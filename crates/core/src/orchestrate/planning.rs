use brewdock_bottle::BottleDownloader;
use brewdock_cellar::{InstalledKeg, find_installed_keg};
use brewdock_formula::{
    Formula, FormulaCache, FormulaRepository, check_supportability, resolve_install_order,
};

use super::{Orchestrator, UpgradeCandidate, is_outdated, pkg_version, request_label};
use crate::BrewdockError;

impl<R: FormulaRepository, D: BottleDownloader> Orchestrator<R, D> {
    /// Resolves the install list: fetch, check supportability, resolve order,
    /// and filter already-installed.
    pub(super) async fn resolve_install_list(
        &self,
        names: &[&str],
    ) -> Result<(Vec<String>, FormulaCache), BrewdockError> {
        let cache = self.fetch_with_deps(names).await?;
        let order = self.resolve_supported_install_order(&cache, names)?;
        Ok((self.filter_pending_formulae(order, |_| false)?, cache))
    }

    /// Validates that all formulae in the cache are installable on the host.
    fn check_cache_supportability(&self, cache: &FormulaCache) -> Result<(), BrewdockError> {
        let host_tag = self.host_tag.as_str();
        for formula in cache.all().values() {
            check_supportability(formula, host_tag)?;
        }
        Ok(())
    }

    /// Resolves install order from a prepared cache after validating supportability.
    pub(super) fn resolve_supported_install_order(
        &self,
        cache: &FormulaCache,
        names: &[&str],
    ) -> Result<Vec<String>, BrewdockError> {
        self.check_cache_supportability(cache)?;
        Ok(resolve_install_order(
            &self.build_install_graph(cache)?,
            names,
        )?)
    }

    /// Builds an upgrade candidate from an installed keg and resolved formula.
    pub(super) fn build_upgrade_candidate(
        &self,
        keg: InstalledKeg,
        formula: Formula,
    ) -> Result<Option<UpgradeCandidate>, BrewdockError> {
        let host_tag = self.host_tag.as_str();
        check_supportability(&formula, host_tag)?;
        let method = self.resolve_install_method(&formula)?;

        let latest_version = pkg_version(&formula.versions.stable, formula.revision);
        if !is_outdated(&keg.pkg_version, &latest_version) {
            return Ok(None);
        }

        Ok(Some(UpgradeCandidate {
            name: keg.name,
            installed_on_request: keg.installed_on_request,
            formula,
            current_version: keg.pkg_version,
            latest_version,
            method,
        }))
    }

    /// Filters install order to formulae not already present in local install state.
    pub(super) fn filter_pending_formulae<F>(
        &self,
        order: Vec<String>,
        mut always_keep: F,
    ) -> Result<Vec<String>, BrewdockError>
    where
        F: FnMut(&str) -> bool,
    {
        let cellar = self.layout.cellar();
        let opt_dir = self.layout.opt_dir();
        let mut pending = Vec::with_capacity(order.len());
        for name in order {
            if always_keep(&name) || find_installed_keg(&name, &cellar, &opt_dir)?.is_none() {
                pending.push(name);
            }
        }
        Ok(pending)
    }

    pub(super) async fn collect_upgrade_candidates(
        &self,
        names: &[&str],
    ) -> Result<Vec<UpgradeCandidate>, BrewdockError> {
        let installed = self.instrument_phase(
            "upgrade-discovery",
            "discover-installed-kegs",
            &request_label(names),
            || self.fetch_installed_kegs(names),
        )?;
        let mut candidates = Vec::with_capacity(installed.len());

        for keg in installed {
            let formula = self.resolve_formula(&keg.name).await?;
            let Some(candidate) = self.build_upgrade_candidate(keg, formula)? else {
                continue;
            };

            if let Err(error) = self
                .instrument_async_phase(
                    "upgrade-discovery",
                    "check-post-install-viability",
                    &candidate.name,
                    self.check_post_install_viability(&candidate.formula),
                )
                .await
            {
                self.emit_warning(
                    "upgrade-discovery",
                    &candidate.name,
                    "skipping upgrade: post_install not supported by bd, use `brew upgrade` instead",
                );
                tracing::warn!(
                    name = %candidate.name,
                    %error,
                    "skipping upgrade: post_install not supported by bd, use `brew upgrade` instead"
                );
                continue;
            }

            candidates.push(candidate);
        }

        Ok(candidates)
    }

    pub(super) async fn resolve_upgrade_install_list(
        &self,
        candidate: &UpgradeCandidate,
    ) -> Result<(Vec<String>, FormulaCache), BrewdockError> {
        let cache = self.fetch_with_deps(&[candidate.name.as_str()]).await?;
        let order = self.resolve_supported_install_order(&cache, &[candidate.name.as_str()])?;
        let candidate_name = candidate.name.as_str();
        Ok((
            self.filter_pending_formulae(order, |name| name == candidate_name)?,
            cache,
        ))
    }
}
