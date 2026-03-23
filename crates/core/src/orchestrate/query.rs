use std::collections::HashSet;

use brewdock_bottle::BottleDownloader;
use brewdock_cellar::{InstalledKeg, find_installed_keg};
use brewdock_formula::{Formula, FormulaRepository, select_bottle};

use super::{
    CleanupResult, DiagnosticCategory, DiagnosticEntry, FormulaInfo, Orchestrator, OutdatedEntry,
    cleanup_directory_tree, is_outdated, pkg_version, unix_now,
};
use crate::BrewdockError;

impl<R: FormulaRepository, D: BottleDownloader> Orchestrator<R, D> {
    /// Searches the cached formula index using an escaped SQL `LIKE` pattern.
    fn search_cached_formulae(&self, pattern: &str) -> Result<Vec<String>, BrewdockError> {
        let escaped = pattern
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let like_pattern = format!("%{escaped}%");
        Ok(self.metadata_store.search_formulae_escaped(&like_pattern)?)
    }

    /// Builds user-facing formula details from a resolved formula and local
    /// filesystem state.
    fn build_formula_info(&self, formula: Formula) -> Result<FormulaInfo, BrewdockError> {
        let host_tag = self.host_tag.as_str();
        let bottle_available = select_bottle(&formula, host_tag).is_some();
        let installed_keg =
            find_installed_keg(&formula.name, &self.layout.cellar(), &self.layout.opt_dir())?;

        Ok(FormulaInfo {
            name: formula.name,
            version: pkg_version(&formula.versions.stable, formula.revision),
            desc: formula.desc,
            homepage: formula.homepage,
            license: formula.license,
            keg_only: formula.keg_only,
            dependencies: formula.dependencies,
            bottle_available,
            installed_version: installed_keg.map(|k| k.pkg_version),
        })
    }

    /// Builds an outdated entry from an installed keg and resolved formula.
    fn build_outdated_entry(keg: InstalledKeg, formula: &Formula) -> Option<OutdatedEntry> {
        let latest_version = pkg_version(&formula.versions.stable, formula.revision);
        is_outdated(&keg.pkg_version, &latest_version).then_some(OutdatedEntry {
            name: keg.name,
            current_version: keg.pkg_version,
            latest_version,
        })
    }

    /// Lists outdated formulae by comparing installed versions against the
    /// metadata cache.
    ///
    /// Reuses the same install-state discovery and metadata layers as `upgrade`.
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if discovery or metadata lookup fails.
    pub async fn outdated(&self, names: &[&str]) -> Result<Vec<OutdatedEntry>, BrewdockError> {
        self.ensure_fresh_cache().await?;
        let installed = self.fetch_installed_kegs(names)?;
        let mut entries = Vec::new();

        for keg in installed {
            let formula = match self.resolve_formula(&keg.name).await {
                Ok(f) => f,
                Err(err) => {
                    self.emit_warning(
                        "outdated",
                        &keg.name,
                        &format!("skipping: cannot resolve formula ({err})"),
                    );
                    tracing::warn!(name = keg.name, %err, "skipping: cannot resolve formula");
                    continue;
                }
            };
            if let Some(entry) = Self::build_outdated_entry(keg, &formula) {
                entries.push(entry);
            }
        }

        Ok(entries)
    }

    /// Searches for formulae by name pattern using the metadata cache.
    ///
    /// The pattern is matched as a substring (wrapped in `%pattern%` for SQL
    /// LIKE). Returns matching formula names sorted alphabetically.
    ///
    /// If the metadata cache is empty, fetches the formula index from the
    /// network first (equivalent to `bd update`).
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if the metadata cache cannot be read or the
    /// network fetch fails.
    pub async fn search(&self, pattern: &str) -> Result<Vec<String>, BrewdockError> {
        self.ensure_fresh_cache().await?;
        self.search_cached_formulae(pattern)
    }

    /// Returns detailed information about a formula.
    ///
    /// Looks up the formula from the metadata cache (falling back to network)
    /// and checks install state from the filesystem.
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if the formula cannot be found or metadata
    /// cannot be read.
    pub async fn info(&self, name: &str) -> Result<FormulaInfo, BrewdockError> {
        let formula = self.resolve_formula(name).await?;
        self.build_formula_info(formula)
    }

    /// Lists all installed formulae from Homebrew-visible filesystem state.
    ///
    /// Reuses the same install-state discovery as `upgrade`.
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if scanning the Cellar fails.
    pub fn list(&self) -> Result<Vec<InstalledKeg>, BrewdockError> {
        self.fetch_installed_kegs(&[])
    }

    /// Cleans up brewdock-owned caches and extracted stores.
    ///
    /// Removes blob files and extracted store directories that are no longer
    /// needed. Does NOT touch `/opt/homebrew/Cellar` or any Homebrew-visible
    /// state.
    ///
    /// If `dry_run` is true, returns the sizes without deleting.
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if cleanup I/O fails.
    pub fn cleanup(&self, dry_run: bool) -> Result<CleanupResult, BrewdockError> {
        let mut result = CleanupResult {
            blobs_removed: 0,
            stores_removed: 0,
            bytes_freed: 0,
        };

        let installed = self.fetch_installed_kegs(&[])?;
        let installed_shas: HashSet<String> = installed
            .iter()
            .filter_map(|keg| {
                let formula = self.metadata_store.load_formula(&keg.name).ok()??;
                let bottle = select_bottle(&formula, self.host_tag.as_str())?;
                Some(bottle.sha256)
            })
            .collect();

        let blob_dir = self.layout.blob_dir();
        if blob_dir.is_dir() {
            let (removed, freed) = cleanup_directory_tree(&blob_dir, &installed_shas, dry_run)?;
            result.blobs_removed = removed;
            result.bytes_freed += freed;
        }

        let store_dir = self.layout.store_dir();
        if store_dir.is_dir() {
            let (removed, freed) = cleanup_directory_tree(&store_dir, &installed_shas, dry_run)?;
            result.stores_removed = removed;
            result.bytes_freed += freed;
        }

        Ok(result)
    }

    /// Runs diagnostic checks on the brewdock environment.
    ///
    /// Checks metadata cache freshness, broken opt symlinks, and missing
    /// receipts. Operates only on brewdock-owned surfaces and
    /// Homebrew-visible install state.
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if diagnostic I/O fails.
    pub fn doctor(&self) -> Result<Vec<DiagnosticEntry>, BrewdockError> {
        let mut diagnostics = Vec::new();

        match self.metadata_store.load_metadata() {
            Ok(Some(meta)) => {
                let now = unix_now().as_secs();
                let age_hours = now.saturating_sub(meta.fetched_at) / 3600;
                if age_hours > 24 {
                    diagnostics.push(DiagnosticEntry {
                        category: DiagnosticCategory::Warning,
                        message: format!(
                            "formula index is {age_hours} hours old ({count} formulae); run `bd update`",
                            count = meta.formula_count,
                        ),
                    });
                } else {
                    diagnostics.push(DiagnosticEntry {
                        category: DiagnosticCategory::Ok,
                        message: format!(
                            "formula index is up to date ({count} formulae, {age_hours}h old)",
                            count = meta.formula_count,
                        ),
                    });
                }
            }
            Ok(None) => diagnostics.push(DiagnosticEntry {
                category: DiagnosticCategory::Warning,
                message: "no formula index cached; run `bd update`".to_owned(),
            }),
            Err(err) => diagnostics.push(DiagnosticEntry {
                category: DiagnosticCategory::Warning,
                message: format!("cannot read formula index: {err}"),
            }),
        }

        let opt_dir = self.layout.opt_dir();
        if opt_dir.is_dir()
            && let Ok(entries) = std::fs::read_dir(&opt_dir)
        {
            let mut broken_count: u32 = 0;
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_symlink() && !path.exists() {
                    broken_count += 1;
                }
            }
            if broken_count > 0 {
                diagnostics.push(DiagnosticEntry {
                    category: DiagnosticCategory::Warning,
                    message: format!("{broken_count} broken symlink(s) in opt directory"),
                });
            }
        }

        let cellar = self.layout.cellar();
        if cellar.is_dir()
            && let Ok(cellar_entries) = std::fs::read_dir(&cellar)
        {
            let valid_kegs: HashSet<String> = self
                .fetch_installed_kegs(&[])?
                .into_iter()
                .map(|k| k.name)
                .collect();
            let mut missing_receipt_count: u32 = 0;
            for entry in cellar_entries.flatten() {
                if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if !valid_kegs.contains(name)
                    && cellar
                        .join(name)
                        .read_dir()
                        .is_ok_and(|mut d| d.next().is_some())
                {
                    missing_receipt_count += 1;
                }
            }
            if missing_receipt_count > 0 {
                diagnostics.push(DiagnosticEntry {
                    category: DiagnosticCategory::Warning,
                    message: format!(
                        "{missing_receipt_count} keg(s) without valid receipt or opt symlink"
                    ),
                });
            }
        }

        Ok(diagnostics)
    }
}
