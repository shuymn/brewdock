use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsStr,
    future::Future,
    path::{Path, PathBuf},
    process::Command,
};

use brewdock_bottle::{BlobStore, BottleDownloader, extract_tar_gz};
use brewdock_cellar::{
    InstallReason, InstallReceipt, InstalledKeg, PostInstallContext, PostInstallTransaction,
    ReceiptDependency, ReceiptSource, ReceiptSourceVersions, RelocationScope,
    atomic_symlink_replace, discover_installed_kegs, find_installed_keg, link, materialize,
    relocate_keg, run_post_install, unlink, validate_post_install, write_receipt,
};
use brewdock_formula::{
    CellarType, FetchOutcome, Formula, FormulaCache, FormulaError, FormulaName, FormulaRepository,
    IndexMetadata, MetadataStore, Requirement, SelectedBottle, UnsupportedReason,
    check_supportability, resolve_install_order, select_bottle,
};
use futures::future::try_join_all;
use tracing::Instrument;

use crate::{BrewdockError, HostTag, Layout, error::SourceBuildError, lock::FileLock};

/// Default tap name for receipt source metadata.
const TAP_NAME: &str = "homebrew/core";

/// Prefix for formula source paths in receipt metadata.
const FORMULA_PATH_PREFIX: &str = "@@HOMEBREW_PREFIX@@/Library/Taps/homebrew/homebrew-core/Formula";

/// Entry in an install plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEntry {
    /// Formula name.
    pub name: String,
    /// Version to install.
    pub version: String,
    /// Resolved install method.
    pub method: InstallMethod,
}

/// Entry in an upgrade plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradePlanEntry {
    /// Formula name.
    pub name: String,
    /// Currently installed version.
    pub from_version: String,
    /// Version to upgrade to.
    pub to_version: String,
    /// Resolved install method for the target version.
    pub method: InstallMethod,
}

/// Resolved install method for a formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallMethod {
    /// Install from a selected bottle.
    Bottle(SelectedBottle),
    /// Install from source using a generic build plan.
    Source(SourceBuildPlan),
}

impl std::fmt::Display for InstallMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bottle(selected) => write!(f, "bottle:{tag}", tag = selected.tag),
            Self::Source(_) => f.write_str("source"),
        }
    }
}

/// Minimal source build plan used for method planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceBuildPlan {
    /// Formula name.
    pub formula_name: String,
    /// Package version string.
    pub version: String,
    /// Source URL.
    pub source_url: String,
    /// Optional source checksum.
    pub source_checksum: Option<String>,
    /// Build dependencies for the source path.
    pub build_dependencies: Vec<String>,
    /// Runtime dependencies for the source path.
    pub runtime_dependencies: Vec<String>,
    /// Installation prefix.
    pub prefix: PathBuf,
    /// Target Cellar path.
    pub cellar_path: PathBuf,
}

/// Result of the prefetch phase for a single formula.
///
/// Contains all downloaded and extracted data needed to finalize
/// the installation without further network access.
enum PrefetchedPayload {
    /// A bottle was downloaded, stored, and extracted.
    Bottle {
        /// Path to the extracted bottle content (`store_dir/sha256/name/version`).
        source_dir: PathBuf,
        /// Target keg path (`Cellar/name/version`).
        keg_path: PathBuf,
        /// Relocation scope determined by the bottle's cellar type.
        relocation_scope: RelocationScope,
    },
    /// A source archive was downloaded and extracted; build is deferred to finalize.
    Source {
        /// Extracted source root directory.
        source_root: PathBuf,
        /// Build plan with all metadata needed for the build.
        plan: SourceBuildPlan,
        /// Tempdir guard — dropped after finalize to clean up extracted source.
        _tempdir: tempfile::TempDir,
    },
}

/// Orchestrates formula installation and upgrade operations.
///
/// Generic over `R` (formula repository) and `D` (bottle downloader) for
/// testability via mock implementations.
///
/// The install pipeline uses [`futures::future::try_join_all`] to prefetch
/// payloads concurrently.  On a `current_thread` runtime, downloads
/// interleave at await points, but blocking I/O (blob store, extraction)
/// runs inline and serialises other in-flight futures for its duration.
/// This is acceptable because the primary latency gain comes from
/// overlapping network requests, not filesystem operations.
pub struct Orchestrator<R, D> {
    repo: R,
    downloader: D,
    layout: Layout,
    host_tag: HostTag,
    metadata_store: MetadataStore,
}

#[derive(Debug, Clone)]
struct UpgradeCandidate {
    name: String,
    installed_on_request: bool,
    formula: Formula,
    current_version: String,
    latest_version: String,
    method: InstallMethod,
}

struct InstallContext<'a, 'b> {
    operation: &'static str,
    requested: &'a HashSet<&'b str>,
    blob_store: &'a BlobStore,
}

/// Bundles a resolved install method with its prefetched payload.
struct ResolvedPayload {
    method: InstallMethod,
    payload: PrefetchedPayload,
}

/// Bundles a resolved install method with its materialized payload.
struct MaterializedFormula {
    method: InstallMethod,
    materialized: MaterializedPayload,
}

/// Bundles method and keg path for finalization.
struct FinalizeContext<'a> {
    method: &'a InstallMethod,
    keg_path: &'a Path,
}

/// Result of the materialize/relocate phase for a single formula.
///
/// Bottles are materialized and relocated in a parallel phase before finalize.
/// Source builds are deferred to the serial finalize phase because they may
/// depend on other formulae being linked first.
enum MaterializedPayload {
    /// A bottle was materialized into its keg and relocated.
    Bottle {
        /// Target keg path (`Cellar/name/version`).
        keg_path: PathBuf,
    },
    /// Source build is deferred to the finalize phase.
    PendingSource(Box<PendingSourcePayload>),
}

/// Payload for a source build that is deferred to the finalize phase.
struct PendingSourcePayload {
    /// Extracted source root directory.
    source_root: PathBuf,
    /// Build plan with all metadata needed for the build.
    plan: SourceBuildPlan,
    /// Tempdir guard — dropped after finalize to clean up extracted source.
    _tempdir: tempfile::TempDir,
}

impl<R: FormulaRepository, D: BottleDownloader> Orchestrator<R, D> {
    /// Creates a new orchestrator.
    #[must_use]
    pub fn new(repo: R, downloader: D, layout: Layout, host_tag: HostTag) -> Self {
        let metadata_store = MetadataStore::new(layout.cache_dir());
        Self {
            repo,
            downloader,
            layout,
            host_tag,
            metadata_store,
        }
    }

    /// Returns the install plan without executing it.
    ///
    /// Resolves dependencies, checks supportability, and filters
    /// already-installed formulae.
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if resolution or validation fails.
    pub async fn plan_install(&self, names: &[&str]) -> Result<Vec<PlanEntry>, BrewdockError> {
        Self::instrument_operation("install-plan", names, async {
            let label = request_label(names);
            let _lock = self.acquire_lock()?;
            let (to_install, cache) = Self::instrument_async_phase(
                "install-plan",
                "resolve-install-list",
                &label,
                self.resolve_install_list(names),
            )
            .await?;
            Self::instrument_phase("install-plan", "resolve-methods", &label, || {
                to_install
                    .iter()
                    .map(|name| {
                        let f = cache.get(name).ok_or_else(|| FormulaError::NotFound {
                            name: FormulaName::from(name.clone()),
                        })?;
                        Ok(PlanEntry {
                            name: name.clone(),
                            version: pkg_version(&f.versions.stable, f.revision),
                            method: self.resolve_install_method(f)?,
                        })
                    })
                    .collect::<Result<Vec<_>, BrewdockError>>()
            })
        })
        .await
    }

    /// Installs the requested formulae and all their dependencies.
    ///
    /// Returns the names of formulae actually installed (excludes already-installed).
    ///
    /// The pipeline is split into three phases:
    /// 1. **Prefetch**: download, verify, store, and extract all payloads
    ///    concurrently. No prefix mutation occurs. Blob store and extract dir
    ///    hits skip download and extraction respectively (warm-path).
    /// 2. **Materialize/Relocate**: copy extracted bottle contents into per-keg
    ///    Cellar paths and patch binaries/text concurrently. Each keg is
    ///    independent so no serialization is needed. Source builds are deferred.
    /// 3. **Finalize**: post-install, link, and write receipts in topological
    ///    order. Source builds run here because they may need linked dependencies.
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if any step fails. Unsupported formulae are
    /// rejected before any download begins.
    pub async fn install(&self, names: &[&str]) -> Result<Vec<String>, BrewdockError> {
        Self::instrument_operation("install", names, async {
            let _lock = self.acquire_lock()?;
            let (to_install, cache) = Self::instrument_async_phase(
                "install",
                "resolve-install-list",
                &request_label(names),
                self.resolve_install_list(names),
            )
            .await?;

            let requested: HashSet<&str> = names.iter().copied().collect();
            let blob_store = BlobStore::new(&self.layout.blob_dir());
            let install_context = InstallContext {
                operation: "install",
                requested: &requested,
                blob_store: &blob_store,
            };

            // Phase 1: Resolve install methods, then prefetch all payloads
            // concurrently (no prefix mutation).
            let resolved_methods: Vec<_> = to_install
                .iter()
                .map(|name| {
                    let formula = cache.get(name).ok_or_else(|| FormulaError::NotFound {
                        name: FormulaName::from(name.clone()),
                    })?;
                    let method =
                        Self::instrument_phase("install", "resolve-install-method", name, || {
                            self.resolve_install_method(formula)
                        })?;
                    Ok((method, formula))
                })
                .collect::<Result<Vec<_>, BrewdockError>>()?;

            let prefetch_futures: Vec<_> = resolved_methods
                .iter()
                .map(|(method, formula)| {
                    Self::instrument_async_phase(
                        "install",
                        "prefetch-payload",
                        &formula.name,
                        self.prefetch_payload("install", formula, method, &blob_store),
                    )
                })
                .collect();

            let payloads = try_join_all(prefetch_futures).await?;

            // Phase 2: Each keg writes only to its own Cellar/name/version
            // and opt/name, so independent kegs can be materialized
            // concurrently via spawn_blocking.
            let opt_dir = self.layout.opt_dir();
            let prefix = self.layout.prefix().to_path_buf();
            let materialize_futures: Vec<_> = resolved_methods
                .iter()
                .zip(payloads)
                .map(|((_, formula), payload)| {
                    let opt_dir = opt_dir.clone();
                    let prefix = prefix.clone();
                    let name = formula.name.clone();
                    async move {
                        let span = tracing::info_span!(
                            "bd.phase",
                            operation = "install",
                            phase = "materialize-payload",
                            target = name.as_str(),
                        );
                        tokio::task::spawn_blocking(move || {
                            let _entered = span.enter();
                            materialize_prefetched_payload(payload, &name, &opt_dir, &prefix)
                        })
                        .await
                        .map_err(|err| BrewdockError::from(std::io::Error::other(err)))?
                    }
                })
                .collect();

            let materialized = try_join_all(materialize_futures).await.inspect_err(|_| {
                // On materialize failure, clean up any kegs that were created.
                for (_, formula) in &resolved_methods {
                    let version = pkg_version(&formula.versions.stable, formula.revision);
                    let keg = self.layout.cellar().join(&formula.name).join(&version);
                    if keg.exists() {
                        let _ = cleanup_failed_install(
                            &keg,
                            self.layout.prefix(),
                            &self.layout.opt_dir(),
                            &formula.name,
                        );
                    }
                }
            })?;

            // Phase 3: Finalize in topological order (prefix-mutating).
            for ((method, formula), materialized_payload) in
                resolved_methods.into_iter().zip(materialized)
            {
                self.finalize_materialized(
                    formula,
                    &cache,
                    &install_context,
                    MaterializedFormula {
                        method,
                        materialized: materialized_payload,
                    },
                )
                .await?;
            }

            Ok(to_install)
        })
        .await
    }

    /// Fetches the formula index and caches it locally.
    ///
    /// Returns the number of formulae cached.
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if the fetch or file write fails.
    pub async fn update(&self) -> Result<usize, BrewdockError> {
        Self::instrument_operation("update", &[], async {
            // Only use ETag for conditional fetch when both metadata and
            // formulae files are present. If the formulae file is missing
            // (e.g., manually deleted), force a full re-fetch to restore
            // integrity rather than accepting a 304 with no local data.
            let existing_meta = self.metadata_store.load_metadata().ok().flatten();
            let existing_etag = existing_meta
                .as_ref()
                .and_then(|m| m.etag.as_deref())
                .filter(|_| self.metadata_store.has_formulae());

            let outcome = Self::instrument_async_phase(
                "update",
                "fetch-formula-index",
                "formula-index",
                self.repo.all_formulae_conditional(existing_etag),
            )
            .await?;

            match outcome {
                FetchOutcome::Modified { formulae, etag } => {
                    self.persist_formula_index(&formulae, etag)?;
                    let count = formulae.len();
                    tracing::info!(count, "formula index cached");
                    Ok(count)
                }
                FetchOutcome::NotModified => {
                    let count = existing_meta.map_or(0, |m| m.formula_count);
                    tracing::info!(count, "formula index unchanged (not modified)");
                    Ok(count)
                }
            }
        })
        .await
    }

    /// Returns the upgrade plan without executing it.
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if resolution or validation fails.
    pub async fn plan_upgrade(
        &self,
        names: &[&str],
    ) -> Result<Vec<UpgradePlanEntry>, BrewdockError> {
        Self::instrument_operation("upgrade-plan", names, async {
            let _lock = self.acquire_lock()?;
            let candidates = Self::instrument_async_phase(
                "upgrade-plan",
                "collect-upgrade-candidates",
                &request_label(names),
                self.collect_upgrade_candidates(names),
            )
            .await?;
            Ok(candidates
                .into_iter()
                .map(|candidate| UpgradePlanEntry {
                    name: candidate.name,
                    from_version: candidate.current_version,
                    to_version: candidate.latest_version,
                    method: candidate.method,
                })
                .collect())
        })
        .await
    }

    /// Upgrades installed formulae to their latest versions.
    ///
    /// If `names` is empty, upgrades all installed formulae. Otherwise,
    /// upgrades only the specified formulae.
    ///
    /// Returns the names of formulae that were actually upgraded.
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if any step fails.
    pub async fn upgrade(&self, names: &[&str]) -> Result<Vec<String>, BrewdockError> {
        Self::instrument_operation("upgrade", names, async {
            let _lock = self.acquire_lock()?;
            let candidates = Self::instrument_async_phase(
                "upgrade",
                "collect-upgrade-candidates",
                &request_label(names),
                self.collect_upgrade_candidates(names),
            )
            .await?;

            let mut upgraded = Vec::new();
            let blob_store = BlobStore::new(&self.layout.blob_dir());

            for candidate in candidates {
                tracing::info!(
                    name = candidate.name,
                    from = candidate.current_version,
                    to = candidate.latest_version,
                    "upgrading formula"
                );

                let old_keg = self
                    .layout
                    .cellar()
                    .join(&candidate.name)
                    .join(&candidate.current_version);
                if old_keg.exists() {
                    Self::instrument_phase("upgrade", "unlink-old-keg", &candidate.name, || {
                        unlink(&old_keg, self.layout.prefix())
                    })?;
                }

                let requested: HashSet<&str> = if candidate.installed_on_request {
                    std::iter::once(candidate.name.as_str()).collect()
                } else {
                    HashSet::new()
                };
                let install_context = InstallContext {
                    operation: "upgrade",
                    requested: &requested,
                    blob_store: &blob_store,
                };

                let install_result = Self::instrument_async_phase(
                    "upgrade",
                    "install-target-version",
                    &candidate.name,
                    self.run_upgrade_install(&candidate, &install_context),
                )
                .await;

                if let Err(error) = install_result {
                    tracing::warn!(
                        name = candidate.name,
                        %error,
                        "upgrade failed, restoring previous version"
                    );
                    self.restore_previous_upgrade(&candidate, &old_keg);
                    return Err(error);
                }

                upgraded.push(candidate.name);
            }

            Ok(upgraded)
        })
        .await
    }

    /// Discovers installed kegs from Homebrew-visible filesystem state.
    ///
    /// If `names` is empty, discovers all installed kegs by scanning the Cellar
    /// directory. Otherwise, looks up only the specified formula names.
    fn fetch_installed_kegs(&self, names: &[&str]) -> Result<Vec<InstalledKeg>, BrewdockError> {
        let cellar = self.layout.cellar();
        let opt_dir = self.layout.opt_dir();
        if names.is_empty() {
            Ok(discover_installed_kegs(&cellar, &opt_dir)?)
        } else {
            let mut kegs = Vec::new();
            for &name in names {
                if let Some(keg) = find_installed_keg(name, &cellar, &opt_dir)? {
                    kegs.push(keg);
                }
            }
            Ok(kegs)
        }
    }

    /// Resolves the install list: fetch, check supportability, resolve order,
    /// and filter already-installed.
    async fn resolve_install_list(
        &self,
        names: &[&str],
    ) -> Result<(Vec<String>, FormulaCache), BrewdockError> {
        let cache = self.fetch_with_deps(names).await?;

        let host_tag = self.host_tag.as_str();
        for formula in cache.all().values() {
            check_supportability(formula, host_tag)?;
        }

        let order = resolve_install_order(&self.build_install_graph(&cache)?, names)?;

        let to_install = {
            let cellar = self.layout.cellar();
            let opt_dir = self.layout.opt_dir();
            let mut pending = Vec::new();
            for name in order {
                if find_installed_keg(&name, &cellar, &opt_dir)?.is_none() {
                    pending.push(name);
                }
            }
            pending
        };

        Ok((to_install, cache))
    }

    /// Persists the formula index and freshness metadata to disk.
    fn persist_formula_index(
        &self,
        formulae: &[Formula],
        etag: Option<String>,
    ) -> Result<(), BrewdockError> {
        Self::instrument_phase(
            "update",
            "persist-formula-index",
            "formula-index",
            || -> Result<(), BrewdockError> {
                self.metadata_store
                    .save_formulae(formulae)
                    .map_err(BrewdockError::from)?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                self.metadata_store
                    .save_metadata(&IndexMetadata {
                        etag,
                        fetched_at: now,
                        formula_count: formulae.len(),
                    })
                    .map_err(BrewdockError::from)?;
                Ok(())
            },
        )
    }

    /// Acquires the brewdock file lock.
    fn acquire_lock(&self) -> Result<FileLock, std::io::Error> {
        FileLock::acquire(&self.layout.lock_dir().join("brewdock.lock"))
    }

    /// Fetches formulae and all transitive dependencies.
    ///
    /// Checks the on-disk metadata cache first and falls back to the
    /// network for any formula not found locally.
    async fn fetch_with_deps(&self, names: &[&str]) -> Result<FormulaCache, BrewdockError> {
        let disk_cache = self.metadata_store.load_formula_map().ok().flatten();

        let mut cache = FormulaCache::new();
        let mut queue: VecDeque<String> = names.iter().map(|name| (*name).to_owned()).collect();

        while let Some(name) = queue.pop_front() {
            if cache.get(&name).is_some() {
                continue;
            }
            let formula = if let Some(f) = disk_cache.as_ref().and_then(|m| m.get(&name)) {
                f.clone()
            } else {
                self.repo.formula(&name).await?
            };
            for dep in self.plan_dependencies(&formula)? {
                if cache.get(&dep).is_none() {
                    queue.push_back(dep);
                }
            }
            cache.insert(formula);
        }

        Ok(cache)
    }

    async fn collect_upgrade_candidates(
        &self,
        names: &[&str],
    ) -> Result<Vec<UpgradeCandidate>, BrewdockError> {
        let installed = Self::instrument_phase(
            "upgrade-discovery",
            "discover-installed-kegs",
            &request_label(names),
            || self.fetch_installed_kegs(names),
        )?;
        let disk_cache = self.metadata_store.load_formula_map().ok().flatten();
        let host_tag = self.host_tag.as_str();
        let mut candidates = Vec::new();

        for keg in installed {
            // Borrow from cache for early checks; only clone if this keg
            // survives the version comparison and becomes a candidate.
            let cached_ref = disk_cache.as_ref().and_then(|m| m.get(&keg.name));
            let fetched;
            let formula_ref = if let Some(f) = cached_ref {
                f
            } else {
                fetched = Self::instrument_async_phase(
                    "upgrade-discovery",
                    "fetch-formula-metadata",
                    &keg.name,
                    self.repo.formula(&keg.name),
                )
                .await?;
                &fetched
            };
            check_supportability(formula_ref, host_tag)?;
            let method = self.resolve_install_method(formula_ref)?;

            let latest_version = pkg_version(&formula_ref.versions.stable, formula_ref.revision);
            if keg.pkg_version == latest_version {
                continue;
            }

            if let Err(error) = Self::instrument_async_phase(
                "upgrade-discovery",
                "check-post-install-viability",
                &keg.name,
                self.check_post_install_viability(formula_ref),
            )
            .await
            {
                tracing::warn!(
                    name = keg.name,
                    %error,
                    "skipping upgrade: post_install not supported by bd, use `brew upgrade` instead"
                );
                continue;
            }

            candidates.push(UpgradeCandidate {
                name: keg.name,
                installed_on_request: keg.installed_on_request,
                formula: formula_ref.clone(),
                current_version: keg.pkg_version,
                latest_version,
                method,
            });
        }

        Ok(candidates)
    }

    async fn resolve_upgrade_install_list(
        &self,
        candidate: &UpgradeCandidate,
    ) -> Result<(Vec<String>, FormulaCache), BrewdockError> {
        let cache = self.fetch_with_deps(&[candidate.name.as_str()]).await?;
        let host_tag = self.host_tag.as_str();
        for formula in cache.all().values() {
            check_supportability(formula, host_tag)?;
        }

        let order = resolve_install_order(
            &self.build_install_graph(&cache)?,
            &[candidate.name.as_str()],
        )?;
        let cellar = self.layout.cellar();
        let opt_dir = self.layout.opt_dir();
        let mut to_install = Vec::new();
        for name in order {
            if name == candidate.name || find_installed_keg(&name, &cellar, &opt_dir)?.is_none() {
                to_install.push(name);
            }
        }
        Ok((to_install, cache))
    }

    /// Runs the install step of an upgrade: resolves deps for source builds,
    /// then prefetches and finalizes each formula in order.
    async fn run_upgrade_install(
        &self,
        candidate: &UpgradeCandidate,
        install_context: &InstallContext<'_, '_>,
    ) -> Result<(), BrewdockError> {
        let (to_install, cache) = if matches!(candidate.method, InstallMethod::Source(_)) {
            self.resolve_upgrade_install_list(candidate).await?
        } else {
            let mut cache = FormulaCache::new();
            cache.insert(candidate.formula.clone());
            (vec![candidate.name.clone()], cache)
        };

        for name in to_install.iter().map(String::as_str) {
            let formula = cache.get(name).ok_or_else(|| FormulaError::NotFound {
                name: FormulaName::from(name),
            })?;
            let method = Self::instrument_phase(
                install_context.operation,
                "resolve-install-method",
                name,
                || self.resolve_install_method(formula),
            )?;
            let payload = Self::instrument_async_phase(
                install_context.operation,
                "prefetch-payload",
                name,
                self.prefetch_payload(
                    install_context.operation,
                    formula,
                    &method,
                    install_context.blob_store,
                ),
            )
            .await?;
            self.finalize_single(
                formula,
                &cache,
                install_context,
                ResolvedPayload { method, payload },
            )
            .await?;
        }
        Ok(())
    }

    /// Prefetches a formula's payload without mutating the prefix.
    ///
    /// For bottles: downloads, verifies, stores in blob store, and extracts.
    /// For source: downloads and extracts the source archive.
    /// No files are written to Cellar, opt, or prefix directories.
    async fn prefetch_payload(
        &self,
        operation: &'static str,
        formula: &Formula,
        method: &InstallMethod,
        blob_store: &BlobStore,
    ) -> Result<PrefetchedPayload, BrewdockError> {
        match method {
            InstallMethod::Bottle(selected_bottle) => {
                let blob_hit =
                    Self::instrument_phase(operation, "check-blob-store", &formula.name, || {
                        blob_store.has(&selected_bottle.sha256)
                    })?;

                if blob_hit {
                    tracing::info!(
                        name = formula.name,
                        sha256 = selected_bottle.sha256,
                        "blob store hit, skipping download"
                    );
                } else {
                    let data = Self::instrument_async_phase(
                        operation,
                        "download-bottle",
                        &formula.name,
                        self.downloader
                            .download_verified(&selected_bottle.url, &selected_bottle.sha256),
                    )
                    .await?;

                    Self::instrument_phase(operation, "store-bottle-blob", &formula.name, || {
                        blob_store.put(&selected_bottle.sha256, &data)
                    })?;
                }

                let version_str = pkg_version(&formula.versions.stable, formula.revision);
                let extract_dir = self.layout.store_dir().join(&selected_bottle.sha256);
                let source_dir = extract_dir.join(&formula.name).join(&version_str);

                if source_dir.exists() {
                    tracing::info!(name = formula.name, "extract dir hit, skipping extraction");
                } else {
                    let blob_path = blob_store.blob_path(&selected_bottle.sha256)?;
                    Self::instrument_phase(operation, "extract-bottle", &formula.name, || {
                        extract_tar_gz(&blob_path, &extract_dir)
                    })?;
                }

                let keg_path = self.layout.cellar().join(&formula.name).join(&version_str);
                let relocation_scope =
                    if matches!(selected_bottle.cellar, CellarType::AnySkipRelocation) {
                        RelocationScope::TextOnly
                    } else {
                        RelocationScope::Full
                    };

                Ok(PrefetchedPayload::Bottle {
                    source_dir,
                    keg_path,
                    relocation_scope,
                })
            }
            InstallMethod::Source(plan) => {
                let checksum = plan.source_checksum.as_deref().ok_or_else(|| {
                    SourceBuildError::MissingSourceChecksum(FormulaName::from(
                        plan.formula_name.clone(),
                    ))
                })?;
                let data = Self::instrument_async_phase(
                    operation,
                    "download-source-archive",
                    &plan.formula_name,
                    self.downloader
                        .download_verified(&plan.source_url, checksum),
                )
                .await?;

                let parent = self.layout.cache_dir().join("sources");
                let formula_name = plan.formula_name.clone();
                let source_url = plan.source_url.clone();
                let span_persist = tracing::info_span!(
                    "bd.phase",
                    operation,
                    phase = "persist-source-archive",
                    target = formula_name.as_str(),
                );
                let span_extract = tracing::info_span!(
                    "bd.phase",
                    operation,
                    phase = "extract-source-archive",
                    target = formula_name.as_str(),
                );

                let (source_root, tempdir) = tokio::task::spawn_blocking(move || {
                    std::fs::create_dir_all(&parent)?;
                    let tempdir = tempfile::tempdir_in(parent)?;
                    let archive_path = tempdir
                        .path()
                        .join(source_archive_filename(&source_url).unwrap_or("source.tar.gz"));
                    {
                        let _entered = span_persist.enter();
                        std::fs::write(&archive_path, &data)?;
                    }
                    let source_root = {
                        let _entered = span_extract.enter();
                        extract_source_archive(&archive_path, tempdir.path())?
                    };
                    Ok::<_, BrewdockError>((source_root, tempdir))
                })
                .await
                .map_err(|err| BrewdockError::from(std::io::Error::other(err)))??;

                Ok(PrefetchedPayload::Source {
                    source_root,
                    plan: plan.clone(),
                    _tempdir: tempdir,
                })
            }
        }
    }

    /// Finalizes a single formula from a materialized payload.
    ///
    /// For bottles: runs post-install, link, and receipt write (keg is already
    /// materialized and relocated).
    /// For source: builds from source first, then runs the same finalize steps.
    async fn finalize_materialized(
        &self,
        formula: &Formula,
        cache: &FormulaCache,
        install_context: &InstallContext<'_, '_>,
        resolved: MaterializedFormula,
    ) -> Result<(), BrewdockError> {
        let name = formula.name.as_str();

        tracing::info!(name, "installing formula");

        let keg_path = match resolved.materialized {
            MaterializedPayload::Bottle { keg_path } => keg_path,
            MaterializedPayload::PendingSource(pending) => {
                self.build_source_to_keg(install_context.operation, name, *pending)?
            }
        };

        self.finalize_with_keg(
            formula,
            cache,
            install_context,
            FinalizeContext {
                method: &resolved.method,
                keg_path: &keg_path,
            },
        )
        .await
    }

    /// Finalizes a single formula from prefetched payload.
    ///
    /// Used by the upgrade path where materialize and finalize are not split.
    async fn finalize_single(
        &self,
        formula: &Formula,
        cache: &FormulaCache,
        install_context: &InstallContext<'_, '_>,
        resolved: ResolvedPayload,
    ) -> Result<(), BrewdockError> {
        let name = formula.name.as_str();

        tracing::info!(name, "installing formula");

        let keg_path = Self::instrument_phase(
            install_context.operation,
            "materialize-payload",
            name,
            || self.materialize_payload(install_context.operation, formula, resolved.payload),
        )
        .inspect_err(|_| {
            let _ = cleanup_failed_install(
                &self
                    .layout
                    .cellar()
                    .join(&formula.name)
                    .join(pkg_version(&formula.versions.stable, formula.revision)),
                self.layout.prefix(),
                &self.layout.opt_dir(),
                &formula.name,
            );
        })?;

        self.finalize_with_keg(
            formula,
            cache,
            install_context,
            FinalizeContext {
                method: &resolved.method,
                keg_path: &keg_path,
            },
        )
        .await
    }

    /// Builds source into a keg and refreshes the opt link.
    fn build_source_to_keg(
        &self,
        operation: &'static str,
        name: &str,
        pending: PendingSourcePayload,
    ) -> Result<PathBuf, BrewdockError> {
        let PendingSourcePayload {
            source_root,
            plan,
            _tempdir: tempdir_guard,
        } = pending;
        Self::instrument_phase(operation, "build-from-source", name, || {
            run_source_build(&source_root, &plan, self.layout.prefix())
        })
        .inspect_err(|_| {
            let _ = cleanup_failed_install(
                &plan.cellar_path,
                self.layout.prefix(),
                &self.layout.opt_dir(),
                name,
            );
        })?;
        // Tempdir kept alive until after build completes; drops here.
        drop(tempdir_guard);
        Self::instrument_phase(operation, "refresh-opt-link", name, || {
            refresh_opt_link(
                &plan.cellar_path,
                &self.layout.opt_dir(),
                &plan.formula_name,
            )
        })?;
        Ok(plan.cellar_path)
    }

    /// Shared finalize logic: post-install, receipt, link, and transaction management.
    async fn finalize_with_keg(
        &self,
        formula: &Formula,
        cache: &FormulaCache,
        install_context: &InstallContext<'_, '_>,
        finalize_ctx: FinalizeContext<'_>,
    ) -> Result<(), BrewdockError> {
        let name = formula.name.as_str();
        let keg_path = finalize_ctx.keg_path;

        let post_install_transaction = Self::instrument_async_phase(
            install_context.operation,
            "post-install",
            name,
            self.execute_post_install(install_context.operation, formula, keg_path),
        )
        .await
        .inspect_err(|_| {
            let _ = cleanup_failed_install(
                keg_path,
                self.layout.prefix(),
                &self.layout.opt_dir(),
                &formula.name,
            );
        })?;

        let is_requested = install_context.requested.contains(formula.name.as_str());
        let receipt = build_receipt(
            finalize_ctx.method,
            if is_requested {
                InstallReason::OnRequest
            } else {
                InstallReason::AsDependency
            },
            Some(unix_timestamp_f64()),
            build_receipt_deps(formula, cache),
            build_receipt_source(formula),
        );
        if let Err(error) =
            Self::instrument_phase(install_context.operation, "finalize-install", name, || {
                self.finalize_installed_formula(formula, keg_path, &receipt)
            })
        {
            if let Some(transaction) = post_install_transaction {
                transaction.rollback()?;
            }
            cleanup_failed_install(
                keg_path,
                self.layout.prefix(),
                &self.layout.opt_dir(),
                &formula.name,
            )?;
            return Err(error);
        }

        if let Some(transaction) = post_install_transaction {
            transaction.commit()?;
        }

        tracing::info!(name, "installation complete");
        Ok(())
    }

    /// Materializes a prefetched payload into the Cellar and relocates it.
    ///
    /// For bottles: copies extracted content to keg and patches binaries.
    /// For source: runs the build and refreshes the opt link.
    fn materialize_payload(
        &self,
        operation: &'static str,
        formula: &Formula,
        payload: PrefetchedPayload,
    ) -> Result<PathBuf, BrewdockError> {
        match payload {
            PrefetchedPayload::Bottle {
                source_dir,
                keg_path,
                relocation_scope,
            } => {
                materialize_and_relocate_bottle(
                    &source_dir,
                    &keg_path,
                    &self.layout.opt_dir(),
                    self.layout.prefix(),
                    &formula.name,
                    relocation_scope,
                )?;
                Ok(keg_path)
            }
            PrefetchedPayload::Source {
                source_root,
                plan,
                _tempdir: tempdir,
            } => self.build_source_to_keg(
                operation,
                &formula.name,
                PendingSourcePayload {
                    source_root,
                    plan,
                    _tempdir: tempdir,
                },
            ),
        }
    }

    /// Checks whether a formula's `post_install` can be parsed and lowered.
    ///
    /// Returns `Ok(())` if the formula has no `post_install` or if it can be
    /// successfully lowered. Returns an error if the source cannot be fetched
    /// or contains unsupported syntax.
    async fn check_post_install_viability(&self, formula: &Formula) -> Result<(), BrewdockError> {
        if let Some(source) = self.fetch_post_install_source(formula).await? {
            validate_post_install(&source, &formula.versions.stable)?;
        }
        Ok(())
    }

    async fn execute_post_install(
        &self,
        operation: &'static str,
        formula: &Formula,
        keg_path: &Path,
    ) -> Result<Option<PostInstallTransaction>, BrewdockError> {
        let Some(ruby_source) = Self::instrument_async_phase(
            operation,
            "fetch-post-install-source",
            &formula.name,
            self.fetch_post_install_source(formula),
        )
        .await?
        else {
            return Ok(None);
        };
        let transaction =
            Self::instrument_phase(operation, "run-post-install", &formula.name, || {
                run_post_install(
                    &ruby_source,
                    &PostInstallContext::new(
                        self.layout.prefix(),
                        keg_path,
                        &formula.versions.stable,
                    ),
                )
            })?;
        Ok(Some(transaction))
    }

    /// Fetches the Ruby source for a formula's `post_install` block.
    ///
    /// Returns `None` if the formula has no `post_install`.
    async fn fetch_post_install_source(
        &self,
        formula: &Formula,
    ) -> Result<Option<String>, BrewdockError> {
        if !formula.post_install_defined {
            return Ok(None);
        }
        let ruby_source_path =
            formula
                .ruby_source_path
                .as_deref()
                .ok_or_else(|| FormulaError::Unsupported {
                    name: FormulaName::from(formula.name.clone()),
                    reason: UnsupportedReason::PostInstallDefined,
                })?;
        let source = self.repo.ruby_source(ruby_source_path).await?;
        Ok(Some(source))
    }

    fn finalize_installed_formula(
        &self,
        formula: &Formula,
        keg_path: &Path,
        receipt: &InstallReceipt,
    ) -> Result<(), BrewdockError> {
        if !formula.keg_only {
            link(keg_path, self.layout.prefix())?;
        }

        write_receipt(keg_path, receipt)?;
        Ok(())
    }

    fn resolve_install_method(&self, formula: &Formula) -> Result<InstallMethod, BrewdockError> {
        let selected = select_bottle(formula, self.host_tag.as_str());

        if let Some(bottle) = selected {
            if bottle.cellar.is_compatible(&self.layout.cellar()) {
                return Ok(InstallMethod::Bottle(bottle));
            }
            tracing::warn!(
                name = formula.name,
                cellar = %bottle.cellar,
                "bottle requires incompatible cellar path, trying source fallback"
            );

            if formula.urls.stable.is_none() {
                return Err(FormulaError::Unsupported {
                    name: FormulaName::from(formula.name.clone()),
                    reason: UnsupportedReason::IncompatibleCellar(bottle.cellar),
                }
                .into());
            }
        } else if formula.urls.stable.is_none() {
            return Err(FormulaError::Unsupported {
                name: FormulaName::from(formula.name.clone()),
                reason: UnsupportedReason::NoBottleForTag(self.host_tag.to_string()),
            }
            .into());
        }

        Ok(InstallMethod::Source(build_source_plan(
            formula,
            &self.layout,
        )?))
    }

    fn plan_dependencies(&self, formula: &Formula) -> Result<Vec<String>, BrewdockError> {
        let mut dependencies = formula.dependencies.clone();
        if matches!(
            self.resolve_install_method(formula)?,
            InstallMethod::Source(_)
        ) {
            dependencies.extend(formula.build_dependencies.iter().cloned());
        }
        Ok(dependencies)
    }

    fn build_install_graph(
        &self,
        cache: &FormulaCache,
    ) -> Result<HashMap<String, Formula>, BrewdockError> {
        cache
            .all()
            .iter()
            .map(|(name, formula)| {
                let mut entry = formula.clone();
                entry.dependencies = self.plan_dependencies(formula)?;
                Ok((name.clone(), entry))
            })
            .collect()
    }
}

fn build_source_plan(
    formula: &Formula,
    layout: &Layout,
) -> Result<SourceBuildPlan, SourceBuildError> {
    if let Some(requirement) = formula.requirements.first() {
        return Err(SourceBuildError::UnsupportedRequirement(
            requirement_name(requirement).to_owned(),
        ));
    }

    let stable = formula
        .urls
        .stable
        .as_ref()
        .ok_or_else(|| SourceBuildError::UnsupportedSourceArchive(formula.name.clone()))?;
    let source_checksum = stable.checksum.clone().ok_or_else(|| {
        SourceBuildError::MissingSourceChecksum(FormulaName::from(formula.name.clone()))
    })?;
    if source_archive_kind(&stable.url).is_none() {
        return Err(SourceBuildError::UnsupportedSourceArchive(
            stable.url.clone(),
        ));
    }
    let version = pkg_version(&formula.versions.stable, formula.revision);
    Ok(SourceBuildPlan {
        formula_name: formula.name.clone(),
        version: version.clone(),
        source_url: stable.url.clone(),
        source_checksum: Some(source_checksum),
        build_dependencies: formula.build_dependencies.clone(),
        runtime_dependencies: formula.dependencies.clone(),
        prefix: layout.prefix().to_path_buf(),
        cellar_path: layout.cellar().join(&formula.name).join(version),
    })
}

fn build_receipt(
    method: &InstallMethod,
    install_reason: InstallReason,
    time: Option<f64>,
    runtime_dependencies: Vec<ReceiptDependency>,
    source: ReceiptSource,
) -> InstallReceipt {
    match method {
        InstallMethod::Bottle(_) => {
            InstallReceipt::for_bottle(install_reason, time, runtime_dependencies, source)
        }
        InstallMethod::Source(_) => {
            InstallReceipt::for_source(install_reason, time, runtime_dependencies, source)
        }
    }
}

fn requirement_name(requirement: &Requirement) -> &str {
    match requirement {
        Requirement::Name(name) => name,
        Requirement::Detailed(detail) => &detail.name,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceArchiveKind {
    TarGz,
}

impl<R: FormulaRepository, D: BottleDownloader> Orchestrator<R, D> {
    async fn instrument_operation<T, F>(
        operation: &'static str,
        request: &[&str],
        future: F,
    ) -> Result<T, BrewdockError>
    where
        F: Future<Output = Result<T, BrewdockError>>,
    {
        future
            .instrument(tracing::info_span!(
                "bd.operation",
                operation,
                request = %request_label(request)
            ))
            .await
    }

    async fn instrument_async_phase<T, E, F>(
        operation: &'static str,
        phase: &'static str,
        target: &str,
        future: F,
    ) -> Result<T, BrewdockError>
    where
        E: Into<BrewdockError>,
        F: Future<Output = Result<T, E>>,
    {
        future
            .instrument(tracing::info_span!("bd.phase", operation, phase, target))
            .await
            .map_err(Into::into)
    }

    fn instrument_phase<T, E, F>(
        operation: &'static str,
        phase: &'static str,
        target: &str,
        work: F,
    ) -> Result<T, BrewdockError>
    where
        E: Into<BrewdockError>,
        F: FnOnce() -> Result<T, E>,
    {
        let span = tracing::info_span!("bd.phase", operation, phase, target);
        let _entered = span.enter();
        work().map_err(Into::into)
    }

    fn restore_previous_upgrade(&self, candidate: &UpgradeCandidate, old_keg: &Path) {
        if !old_keg.exists() {
            return;
        }

        if !candidate.formula.keg_only {
            let _ = link(old_keg, self.layout.prefix());
        }
        let _ = refresh_opt_link(old_keg, &self.layout.opt_dir(), &candidate.name);
    }
}

/// Copies extracted bottle content into a keg and patches binaries/text.
///
/// Shared by both the parallel install path and the serial upgrade path.
#[expect(
    clippy::too_many_arguments,
    reason = "thin delegation to materialize + relocate_keg"
)]
fn materialize_and_relocate_bottle(
    source_dir: &Path,
    keg_path: &Path,
    opt_dir: &Path,
    prefix: &Path,
    formula_name: &str,
    relocation_scope: RelocationScope,
) -> Result<(), BrewdockError> {
    materialize(source_dir, keg_path, opt_dir, formula_name)?;
    relocate_keg(keg_path, prefix, relocation_scope)?;
    Ok(())
}

/// Materializes a prefetched payload into a [`MaterializedPayload`].
///
/// This is a free function (not a method) so it can be moved into
/// [`tokio::task::spawn_blocking`] without borrowing the orchestrator.
fn materialize_prefetched_payload(
    payload: PrefetchedPayload,
    formula_name: &str,
    opt_dir: &Path,
    prefix: &Path,
) -> Result<MaterializedPayload, BrewdockError> {
    match payload {
        PrefetchedPayload::Bottle {
            source_dir,
            keg_path,
            relocation_scope,
        } => {
            materialize_and_relocate_bottle(
                &source_dir,
                &keg_path,
                opt_dir,
                prefix,
                formula_name,
                relocation_scope,
            )?;
            Ok(MaterializedPayload::Bottle { keg_path })
        }
        PrefetchedPayload::Source {
            source_root,
            plan,
            _tempdir: tempdir,
        } => Ok(MaterializedPayload::PendingSource(Box::new(
            PendingSourcePayload {
                source_root,
                plan,
                _tempdir: tempdir,
            },
        ))),
    }
}

fn request_label(names: &[&str]) -> String {
    if names.is_empty() {
        "all".to_owned()
    } else {
        names.join(",")
    }
}

fn source_archive_filename(url: &str) -> Option<&str> {
    let trimmed = url.split('?').next().unwrap_or(url);
    trimmed.rsplit('/').next()
}

fn source_archive_kind(url: &str) -> Option<SourceArchiveKind> {
    let filename = source_archive_filename(url)?.to_ascii_lowercase();
    if filename.ends_with(".tar.gz")
        || Path::new(&filename)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("tgz"))
    {
        Some(SourceArchiveKind::TarGz)
    } else {
        None
    }
}

fn extract_source_archive(
    archive_path: &Path,
    tempdir_root: &Path,
) -> Result<PathBuf, BrewdockError> {
    let kind = source_archive_kind(
        archive_path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default(),
    )
    .ok_or_else(|| {
        SourceBuildError::UnsupportedSourceArchive(archive_path.display().to_string())
    })?;

    let extract_dir = tempdir_root.join("extract");
    match kind {
        SourceArchiveKind::TarGz => extract_tar_gz(archive_path, &extract_dir)?,
    }
    discover_source_root(&extract_dir)
}

fn discover_source_root(extract_dir: &Path) -> Result<PathBuf, BrewdockError> {
    let entries: Vec<PathBuf> = std::fs::read_dir(extract_dir)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|entry| entry.path())
        .collect();

    if entries.len() == 1 && entries[0].is_dir() {
        Ok(entries[0].clone())
    } else if entries.is_empty() {
        Err(SourceBuildError::MissingSourceRoot(extract_dir.display().to_string()).into())
    } else {
        Ok(extract_dir.to_path_buf())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceBuildSystem {
    Configure,
    Cmake,
    Meson,
    PerlMakeMaker,
    Make,
}

fn run_source_build(
    source_root: &Path,
    plan: &SourceBuildPlan,
    prefix: &Path,
) -> Result<(), BrewdockError> {
    std::fs::create_dir_all(&plan.cellar_path)?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    let joined_path = std::env::join_paths(
        std::iter::once(prefix.join("bin")).chain(std::env::split_paths(&path)),
    )
    .map_err(|error| std::io::Error::other(error.to_string()))?;
    let build_system = detect_build_system(source_root)?;
    let prefix_arg = format!("--prefix={}", plan.cellar_path.display());

    match build_system {
        SourceBuildSystem::Configure => {
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "./configure",
                &[&prefix_arg],
            )?;
            run_build_command(source_root, prefix, &joined_path, "make", &[])?;
            run_build_command(source_root, prefix, &joined_path, "make", &["install"])?;
        }
        SourceBuildSystem::Cmake => {
            let build_dir = source_root.join("build");
            let configure_args = cmake_configure_args(source_root, &build_dir, &plan.cellar_path);
            let configure_arg_refs = configure_args
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "cmake",
                &configure_arg_refs,
            )?;
            let build_arg = build_dir.display().to_string();
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "cmake",
                &["--build", &build_arg],
            )?;
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "cmake",
                &["--install", &build_arg],
            )?;
        }
        SourceBuildSystem::Meson => {
            let build_dir = source_root.join("build");
            let build_arg = build_dir.display().to_string();
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "meson",
                &["setup", &build_arg, &prefix_arg],
            )?;
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "ninja",
                &["-C", &build_arg],
            )?;
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "ninja",
                &["-C", &build_arg, "install"],
            )?;
        }
        SourceBuildSystem::PerlMakeMaker => {
            let install_base = format!("INSTALL_BASE={}", plan.cellar_path.display());
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "perl",
                &["Makefile.PL", &install_base],
            )?;
            run_build_command(source_root, prefix, &joined_path, "make", &[])?;
            run_build_command(source_root, prefix, &joined_path, "make", &["install"])?;
        }
        SourceBuildSystem::Make => {
            let prefix_value = plan.cellar_path.display().to_string();
            run_build_command(source_root, prefix, &joined_path, "make", &[])?;
            run_build_command(
                source_root,
                prefix,
                &joined_path,
                "make",
                &[
                    "install",
                    &format!("PREFIX={prefix_value}"),
                    &format!("prefix={prefix_value}"),
                ],
            )?;
        }
    }

    Ok(())
}

fn cmake_configure_args(source_root: &Path, build_dir: &Path, cellar_path: &Path) -> Vec<String> {
    vec![
        "-S".to_owned(),
        source_root.display().to_string(),
        "-B".to_owned(),
        build_dir.display().to_string(),
        format!("-DCMAKE_INSTALL_PREFIX={}", cellar_path.display()),
    ]
}

fn detect_build_system(source_root: &Path) -> Result<SourceBuildSystem, BrewdockError> {
    let candidates = [
        ("Makefile.PL", SourceBuildSystem::PerlMakeMaker),
        ("CMakeLists.txt", SourceBuildSystem::Cmake),
        ("meson.build", SourceBuildSystem::Meson),
        ("configure", SourceBuildSystem::Configure),
        ("Makefile", SourceBuildSystem::Make),
        ("makefile", SourceBuildSystem::Make),
    ];

    candidates
        .iter()
        .find(|(name, _)| source_root.join(name).exists())
        .map(|(_, build_system)| *build_system)
        .ok_or_else(|| {
            SourceBuildError::UnsupportedBuildSystem(source_root.display().to_string()).into()
        })
}

fn run_build_command(
    source_root: &Path,
    prefix: &Path,
    path: &OsStr,
    program: &str,
    args: &[&str],
) -> Result<(), BrewdockError> {
    let output = Command::new(program)
        .current_dir(source_root)
        .env("PATH", path)
        .env("HOMEBREW_PREFIX", prefix)
        .args(args)
        .output()
        .map_err(|error| SourceBuildError::CommandFailed {
            command: format_command(program, args),
            stderr: error.to_string(),
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    Err(SourceBuildError::CommandFailed {
        command: format_command(program, args),
        stderr: if detail.is_empty() {
            output.status.code().map_or_else(
                || "terminated by signal".to_owned(),
                |code| code.to_string(),
            )
        } else {
            detail
        },
    }
    .into())
}

fn format_command(program: &str, args: &[&str]) -> String {
    if args.is_empty() {
        program.to_owned()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

fn refresh_opt_link(
    keg_path: &Path,
    opt_dir: &Path,
    formula_name: &str,
) -> Result<(), BrewdockError> {
    std::fs::create_dir_all(opt_dir)?;
    let opt_link = opt_dir.join(formula_name);
    atomic_symlink_replace(keg_path, &opt_link)?;
    Ok(())
}

fn cleanup_failed_install(
    keg_path: &Path,
    prefix: &Path,
    opt_dir: &Path,
    formula_name: &str,
) -> Result<(), BrewdockError> {
    let _ = unlink(keg_path, prefix);
    let opt_link = opt_dir.join(formula_name);
    if opt_link.symlink_metadata().is_ok() {
        std::fs::remove_file(&opt_link)?;
    }

    if keg_path.exists() {
        std::fs::remove_dir_all(keg_path)?;
    }

    // Clean up temp keg left by an interrupted atomic materialize.
    if let Some(parent) = keg_path.parent()
        && let Some(version) = keg_path.file_name().and_then(|n| n.to_str())
    {
        let temp_keg = parent.join(format!(".{version}.brewdock-tmp"));
        if temp_keg.exists() {
            std::fs::remove_dir_all(&temp_keg)?;
        }
    }

    Ok(())
}

/// Creates the package version string used in Cellar paths.
fn pkg_version(version: &str, revision: u32) -> String {
    if revision > 0 {
        format!("{version}_{revision}")
    } else {
        version.to_owned()
    }
}

/// Builds receipt dependency list from formula dependencies.
fn build_receipt_deps(formula: &Formula, cache: &FormulaCache) -> Vec<ReceiptDependency> {
    formula
        .dependencies
        .iter()
        .filter_map(|dep_name| cache.get(dep_name))
        .map(|dep| ReceiptDependency {
            full_name: dep.full_name.clone(),
            version: dep.versions.stable.clone(),
            revision: dep.revision,
            pkg_version: pkg_version(&dep.versions.stable, dep.revision),
            declared_directly: true,
        })
        .collect()
}

/// Builds receipt source from formula metadata.
fn build_receipt_source(formula: &Formula) -> ReceiptSource {
    let first_char = formula.name.chars().next().unwrap_or('_');
    ReceiptSource {
        path: format!("{FORMULA_PATH_PREFIX}/{first_char}/{}.rb", formula.name),
        tap: TAP_NAME.to_owned(),
        spec: "stable".to_owned(),
        versions: ReceiptSourceVersions {
            stable: formula.versions.stable.clone(),
            head: formula.versions.head.clone(),
            version_scheme: 0,
        },
    }
}

/// Returns the current duration since Unix epoch.
fn unix_now() -> std::time::Duration {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
}

/// Returns the current Unix timestamp as `f64` seconds.
fn unix_timestamp_f64() -> f64 {
    unix_now().as_secs_f64()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        thread,
        time::Duration,
    };

    use brewdock_bottle::BottleError;
    use brewdock_cellar::find_installed_keg;
    use brewdock_formula::{
        BottleFile, BottleSpec, BottleStable, CellarType, FormulaUrls, StableUrl, Versions,
    };

    use super::*;

    const HOST_TAG: &str = "arm64_sequoia";
    const PLAN_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    // --- Mock types ---

    struct MockRepo {
        formulae: HashMap<String, Formula>,
        ruby_sources: HashMap<String, String>,
    }

    impl MockRepo {
        fn new(list: Vec<Formula>) -> Self {
            let formulae = list.into_iter().map(|f| (f.name.clone(), f)).collect();
            Self {
                formulae,
                ruby_sources: HashMap::new(),
            }
        }

        fn with_sources(list: Vec<Formula>, ruby_sources: HashMap<String, String>) -> Self {
            let formulae = list.into_iter().map(|f| (f.name.clone(), f)).collect();
            Self {
                formulae,
                ruby_sources,
            }
        }
    }

    impl FormulaRepository for MockRepo {
        async fn formula(&self, name: &str) -> Result<Formula, FormulaError> {
            self.formulae
                .get(name)
                .cloned()
                .ok_or_else(|| FormulaError::NotFound {
                    name: FormulaName::from(name),
                })
        }

        async fn all_formulae(&self) -> Result<Vec<Formula>, FormulaError> {
            Ok(self.formulae.values().cloned().collect())
        }

        async fn ruby_source(&self, ruby_source_path: &str) -> Result<String, FormulaError> {
            self.ruby_sources
                .get(ruby_source_path)
                .cloned()
                .ok_or_else(|| FormulaError::NotFound {
                    name: FormulaName::from(ruby_source_path),
                })
        }
    }

    struct MockDownloader {
        data: HashMap<String, Vec<u8>>,
        download_count: Arc<AtomicUsize>,
    }

    impl MockDownloader {
        fn new(entries: Vec<(&str, Vec<u8>)>, counter: Arc<AtomicUsize>) -> Self {
            let data = entries
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect();
            Self {
                data,
                download_count: counter,
            }
        }
    }

    impl BottleDownloader for MockDownloader {
        async fn download_verified(
            &self,
            _url: &str,
            expected_sha256: &str,
        ) -> Result<Vec<u8>, BottleError> {
            self.download_count.fetch_add(1, Ordering::SeqCst);
            self.data.get(expected_sha256).cloned().ok_or_else(|| {
                BottleError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("mock: no data for {expected_sha256}"),
                ))
            })
        }
    }

    // --- Helpers ---

    fn make_formula(name: &str, version: &str, deps: &[&str], sha256: &str) -> Formula {
        Formula {
            name: name.to_owned(),
            full_name: name.to_owned(),
            versions: Versions {
                stable: version.to_owned(),
                head: None,
                bottle: true,
            },
            revision: 0,
            ruby_source_path: Some(format!("Formula/{name}.rb")),
            bottle: BottleSpec {
                stable: Some(BottleStable {
                    rebuild: 0,
                    root_url: "https://example.com".to_owned(),
                    files: HashMap::from([(
                        HOST_TAG.to_owned(),
                        BottleFile {
                            cellar: CellarType::Any,
                            url: format!("https://example.com/{name}.tar.gz"),
                            sha256: sha256.to_owned(),
                        },
                    )]),
                }),
            },
            urls: FormulaUrls {
                stable: Some(StableUrl {
                    url: format!("https://example.com/{name}-{version}.tar.gz"),
                    checksum: Some(sha256.to_owned()),
                }),
            },
            pour_bottle_only_if: None,
            keg_only: false,
            dependencies: deps.iter().map(|s| (*s).to_owned()).collect(),
            build_dependencies: Vec::new(),
            uses_from_macos: Vec::new(),
            requirements: Vec::new(),
            disabled: false,
            post_install_defined: false,
        }
    }

    /// Creates a tar.gz archive mimicking a Homebrew bottle structure.
    fn create_bottle_tar_gz(
        name: &str,
        version: &str,
        files: &[(&str, &[u8])],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use flate2::{Compression, write::GzEncoder};

        let buf = Vec::new();
        let encoder = GzEncoder::new(buf, Compression::default());
        let mut builder = tar::Builder::new(encoder);

        for &(path, contents) in files {
            let full_path = format!("{name}/{version}/{path}");
            let mut header = tar::Header::new_gnu();
            header.set_path(&full_path)?;
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, contents)?;
        }

        let encoder = builder.into_inner()?;
        let compressed = encoder.finish()?;
        Ok(compressed)
    }

    fn create_source_tar_gz(
        root: &str,
        files: &[(&str, &[u8])],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use flate2::{Compression, write::GzEncoder};

        let buf = Vec::new();
        let encoder = GzEncoder::new(buf, Compression::default());
        let mut builder = tar::Builder::new(encoder);

        for &(path, contents) in files {
            let full_path = format!("{root}/{path}");
            let mut header = tar::Header::new_gnu();
            header.set_path(&full_path)?;
            header.set_size(contents.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append(&header, contents)?;
        }

        let encoder = builder.into_inner()?;
        let compressed = encoder.finish()?;
        Ok(compressed)
    }

    fn create_source_tar_gz_with_raw_paths(
        entries: &[(&[u8], &[u8])],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use flate2::{Compression, write::GzEncoder};
        use tar::{Builder, Header};

        let buf = Vec::new();
        let encoder = GzEncoder::new(buf, Compression::default());
        let mut builder = Builder::new(encoder);

        for &(path, contents) in entries {
            let mut header = Header::new_gnu();
            header.as_mut_bytes()[..100].fill(0);
            header.as_mut_bytes()[..path.len()].copy_from_slice(path);
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, contents)?;
        }

        let encoder = builder.into_inner()?;
        Ok(encoder.finish()?)
    }

    #[test]
    fn test_cmake_configure_args_use_cmake_install_prefix() {
        let source_root = Path::new("/tmp/source");
        let build_dir = Path::new("/tmp/source/build");
        let cellar_path = Path::new("/opt/homebrew/Cellar/demo/1.0");

        let args = cmake_configure_args(source_root, build_dir, cellar_path);

        assert!(args.iter().any(|arg| arg == "-S"));
        assert!(args.iter().any(|arg| arg == "-B"));
        assert!(
            args.iter()
                .any(|arg| { arg == "-DCMAKE_INSTALL_PREFIX=/opt/homebrew/Cellar/demo/1.0" })
        );
        assert!(
            !args
                .iter()
                .any(|arg| arg == "--prefix=/opt/homebrew/Cellar/demo/1.0")
        );
    }

    fn make_orchestrator(
        formulae: Vec<Formula>,
        bottles: Vec<(&str, Vec<u8>)>,
        counter: Arc<AtomicUsize>,
        layout: Layout,
    ) -> Result<Orchestrator<MockRepo, MockDownloader>, BrewdockError> {
        let host_tag: HostTag = HOST_TAG.parse()?;
        let repo = MockRepo::new(formulae);
        let downloader = MockDownloader::new(bottles, counter);
        Ok(Orchestrator::new(repo, downloader, layout, host_tag))
    }

    fn make_orchestrator_with_sources(
        formulae: Vec<Formula>,
        ruby_sources: HashMap<String, String>,
        bottles: Vec<(&str, Vec<u8>)>,
        counter: Arc<AtomicUsize>,
        layout: Layout,
    ) -> Result<Orchestrator<MockRepo, MockDownloader>, BrewdockError> {
        let host_tag: HostTag = HOST_TAG.parse()?;
        let repo = MockRepo::with_sources(formulae, ruby_sources);
        let downloader = MockDownloader::new(bottles, counter);
        Ok(Orchestrator::new(repo, downloader, layout, host_tag))
    }

    fn move_host_bottle_to_tag(
        formula: &mut Formula,
        tag: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let stable = formula
            .bottle
            .stable
            .as_mut()
            .ok_or("expected stable bottle metadata")?;
        let bottle = stable
            .files
            .remove(HOST_TAG)
            .ok_or("expected host bottle")?;
        stable.files.insert(tag.to_owned(), bottle);
        Ok(())
    }

    /// Sets up filesystem install state for a formula (keg directory + receipt + opt symlink).
    fn setup_installed_keg(
        layout: &Layout,
        name: &str,
        version: &str,
        on_request: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let keg_path = layout.cellar().join(name).join(version);
        std::fs::create_dir_all(&keg_path)?;

        let receipt = InstallReceipt::for_bottle(
            if on_request {
                InstallReason::OnRequest
            } else {
                InstallReason::AsDependency
            },
            Some(1_000.0),
            Vec::new(),
            ReceiptSource {
                path: String::new(),
                tap: "homebrew/core".to_owned(),
                spec: "stable".to_owned(),
                versions: ReceiptSourceVersions {
                    stable: version.to_owned(),
                    head: None,
                    version_scheme: 0,
                },
            },
        );
        write_receipt(&keg_path, &receipt)?;

        // Create opt symlink (absolute path; relative_from_to is pub(crate) in cellar).
        std::fs::create_dir_all(layout.opt_dir())?;
        let opt_link = layout.opt_dir().join(name);
        atomic_symlink_replace(&keg_path, &opt_link)?;
        Ok(())
    }

    fn create_simple_bottle(
        name: &str,
        version: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        create_bottle_tar_gz(name, version, &[("bin/tool", b"#!/bin/sh\necho ok\n")])
    }

    /// Asserts that a formula is visible as installed via filesystem state.
    fn assert_installed(layout: &Layout, name: &str) {
        assert!(
            find_installed_keg(name, &layout.cellar(), &layout.opt_dir())
                .ok()
                .flatten()
                .is_some(),
            "expected {name} to be installed"
        );
    }

    /// Asserts that a formula is NOT visible as installed via filesystem state.
    fn assert_not_installed(layout: &Layout, name: &str) {
        assert!(
            find_installed_keg(name, &layout.cellar(), &layout.opt_dir())
                .ok()
                .flatten()
                .is_none(),
            "expected {name} to not be installed"
        );
    }

    // --- Install tests ---

    #[tokio::test]
    async fn test_install_resolves_and_installs_in_topological_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let sha_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let formula_a = make_formula("a", "1.0", &["b"], sha_a);
        let formula_b = make_formula("b", "2.0", &[], sha_b);

        let tar_a = create_bottle_tar_gz("a", "1.0", &[("bin/a_tool", b"#!/bin/sh\necho a")])?;
        let tar_b = create_bottle_tar_gz("b", "2.0", &[("bin/b_tool", b"#!/bin/sh\necho b")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula_a, formula_b],
            vec![(sha_a, tar_a), (sha_b, tar_b)],
            counter.clone(),
            layout.clone(),
        )?;

        let installed = orchestrator.install(&["a"]).await?;

        // "b" must be installed before "a" (topological order).
        assert_eq!(installed, vec!["b", "a"]);

        // Verify files exist in Cellar.
        assert!(layout.cellar().join("a/1.0/bin/a_tool").exists());
        assert!(layout.cellar().join("b/2.0/bin/b_tool").exists());

        // Verify symlinks in prefix.
        let prefix = layout.prefix();
        assert!(prefix.join("bin/a_tool").is_symlink());
        assert!(prefix.join("bin/b_tool").is_symlink());

        // Verify opt symlinks.
        assert!(layout.opt_dir().join("a").is_symlink());
        assert!(layout.opt_dir().join("b").is_symlink());

        // Verify receipts.
        assert!(layout.cellar().join("a/1.0/INSTALL_RECEIPT.json").exists());
        assert!(layout.cellar().join("b/2.0/INSTALL_RECEIPT.json").exists());

        // Verify filesystem install state.
        let keg_a =
            find_installed_keg("a", &layout.cellar(), &layout.opt_dir())?.ok_or("expected a")?;
        assert_eq!(keg_a.pkg_version, "1.0");
        assert!(keg_a.installed_on_request);
        let keg_b =
            find_installed_keg("b", &layout.cellar(), &layout.opt_dir())?.ok_or("expected b")?;
        assert_eq!(keg_b.pkg_version, "2.0");
        assert!(!keg_b.installed_on_request);

        // Both bottles downloaded.
        assert_eq!(counter.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[tokio::test]
    async fn test_install_skips_already_installed() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let sha_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let formula_a = make_formula("a", "1.0", &["b"], sha_a);
        let formula_b = make_formula("b", "2.0", &[], sha_b);

        let tar_a = create_bottle_tar_gz("a", "1.0", &[("bin/a_tool", b"#!/bin/sh\necho a")])?;
        let tar_b = create_bottle_tar_gz("b", "2.0", &[("bin/b_tool", b"#!/bin/sh\necho b")])?;

        // Pre-populate filesystem state with "b" already installed.
        setup_installed_keg(&layout, "b", "2.0", false)?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula_a, formula_b],
            vec![(sha_a, tar_a), (sha_b, tar_b)],
            counter.clone(),
            layout.clone(),
        )?;

        let installed = orchestrator.install(&["a"]).await?;

        // Only "a" should be installed; "b" is skipped.
        assert_eq!(installed, vec!["a"]);

        // Only one download (for "a").
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn test_plan_install_uses_compatible_bottle_method()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let mut formula = make_formula("a", "1.0", &[], PLAN_SHA);
        move_host_bottle_to_tag(&mut formula, "arm64_sonoma")?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout)?;

        let plan = orchestrator.plan_install(&["a"]).await?;
        let entry = plan.first().ok_or("expected install plan entry")?;
        assert!(matches!(
            &entry.method,
            InstallMethod::Bottle(selected) if selected.tag == "arm64_sonoma"
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_plan_install_uses_source_method_when_bottle_missing()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let mut formula = make_formula("a", "1.0", &[], PLAN_SHA);
        formula.versions.bottle = false;
        formula.bottle.stable = None;
        formula.build_dependencies = vec!["pkgconf".to_owned()];
        let pkgconf = make_formula(
            "pkgconf",
            "2.0",
            &[],
            "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
        );

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula, pkgconf], vec![], counter, layout.clone())?;

        let plan = orchestrator.plan_install(&["a"]).await?;
        let entry = plan
            .iter()
            .find(|entry| entry.name == "a")
            .ok_or("expected install plan entry")?;
        assert!(matches!(
            &entry.method,
            InstallMethod::Source(source)
                if source.formula_name == "a"
                    && source.prefix == layout.prefix()
                    && source.cellar_path == layout.cellar().join("a/1.0")
                    && source.build_dependencies == vec!["pkgconf".to_owned()]
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_plan_install_keeps_post_install_formula_plannable()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let mut formula = make_formula("a", "1.0", &[], PLAN_SHA);
        formula.post_install_defined = true;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout)?;

        let plan = orchestrator.plan_install(&["a"]).await?;
        let entry = plan.first().ok_or("expected install plan entry")?;
        assert!(matches!(&entry.method, InstallMethod::Bottle(_)));
        Ok(())
    }

    #[tokio::test]
    async fn test_plan_upgrade_reuses_install_method_resolution()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        setup_installed_keg(&layout, "a", "1.0", true)?;

        let mut formula = make_formula("a", "2.0", &[], PLAN_SHA);
        move_host_bottle_to_tag(&mut formula, "arm64_sonoma")?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout)?;

        let plan = orchestrator.plan_upgrade(&["a"]).await?;
        let entry = plan.first().ok_or("expected upgrade plan entry")?;
        assert!(matches!(
            &entry.method,
            InstallMethod::Bottle(selected) if selected.tag == "arm64_sonoma"
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_install_rejects_unsupported_before_download()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let mut formula = make_formula(
            "disabled_pkg",
            "1.0",
            &[],
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        );
        formula.disabled = true;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula], vec![], counter.clone(), layout.clone())?;

        let result = orchestrator.install(&["disabled_pkg"]).await;

        assert!(result.is_err());

        // No downloads should have been attempted.
        assert_eq!(counter.load(Ordering::SeqCst), 0);

        // No Cellar directory should exist.
        assert!(!layout.cellar().join("disabled_pkg").exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_install_runs_post_install_before_link_and_persists_receipt_and_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let mut formula = make_formula("demo", "1.0", &[], sha);
        formula.post_install_defined = true;

        let tar = create_bottle_tar_gz(
            "demo",
            "1.0",
            &[
                (
                    "bin/write-flag",
                    b"#!/bin/sh\nprintf '%s' \"$1\" > \"$2\"\n",
                ),
                ("share/src.txt", b"payload"),
            ],
        )?;

        let source = r#"
class Demo < Formula
  def post_install
    (var/"demo").mkpath
    cp share/"src.txt", var/"demo/copied.txt"
    system "/bin/sh", bin/"write-flag", "done", var/"demo/result.txt"
  end
end
"#;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator_with_sources(
            vec![formula],
            HashMap::from([("Formula/demo.rb".to_owned(), source.to_owned())]),
            vec![(sha, tar)],
            counter,
            layout.clone(),
        )?;

        let installed = orchestrator.install(&["demo"]).await?;

        assert_eq!(installed, vec!["demo"]);
        assert_eq!(
            std::fs::read_to_string(layout.prefix().join("var/demo/copied.txt"))?,
            "payload"
        );
        assert_eq!(
            std::fs::read_to_string(layout.prefix().join("var/demo/result.txt"))?,
            "done"
        );
        assert!(layout.prefix().join("bin/write-flag").is_symlink());
        assert!(
            layout
                .cellar()
                .join("demo/1.0/INSTALL_RECEIPT.json")
                .exists()
        );
        assert_installed(&layout, "demo");
        Ok(())
    }

    #[tokio::test]
    async fn test_install_cleans_up_failed_post_install_without_receipt_or_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        let mut formula = make_formula("demo", "1.0", &[], sha);
        formula.post_install_defined = true;

        let tar = create_bottle_tar_gz("demo", "1.0", &[("share/src.txt", b"payload")])?;
        let source = r#"
class Demo < Formula
  def post_install
    unsupported_call "boom"
  end
end
"#;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator_with_sources(
            vec![formula],
            HashMap::from([("Formula/demo.rb".to_owned(), source.to_owned())]),
            vec![(sha, tar)],
            counter,
            layout.clone(),
        )?;

        let result = orchestrator.install(&["demo"]).await;

        assert!(matches!(
            result,
            Err(BrewdockError::Cellar(
                brewdock_cellar::CellarError::UnsupportedPostInstallSyntax { .. }
            ))
        ));
        assert!(!layout.cellar().join("demo/1.0").exists());
        assert!(layout.opt_dir().join("demo").symlink_metadata().is_err());
        assert!(
            !layout
                .cellar()
                .join("demo/1.0/INSTALL_RECEIPT.json")
                .exists()
        );
        assert_not_installed(&layout, "demo");
        assert!(!layout.prefix().join("bin").exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_install_cleans_up_when_ruby_source_fetch_fails()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha = "1212121212121212121212121212121212121212121212121212121212121212";
        let mut formula = make_formula("demo", "1.0", &[], sha);
        formula.post_install_defined = true;

        let orchestrator = make_orchestrator(
            vec![formula],
            vec![(
                sha,
                create_bottle_tar_gz("demo", "1.0", &[("bin/demo", b"binary")])?,
            )],
            Arc::new(AtomicUsize::new(0)),
            layout.clone(),
        )?;

        let result = orchestrator.install(&["demo"]).await;

        assert!(matches!(
            result,
            Err(BrewdockError::Formula(FormulaError::NotFound { .. }))
        ));
        assert!(!layout.cellar().join("demo/1.0").exists());
        assert!(layout.opt_dir().join("demo").symlink_metadata().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn test_install_bootstraps_certificate_bundle_post_install_pattern()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let mut formula = make_formula("ca-certificates", "1.0", &[], sha);
        formula.post_install_defined = true;

        let tar = create_bottle_tar_gz(
            "ca-certificates",
            "1.0",
            &[("share/ca-certificates/cacert.pem", b"bundle")],
        )?;
        let source = r#"
class CaCertificates < Formula
  def post_install
    if OS.mac?
      macos_post_install
    else
      linux_post_install
    end
  end

  def macos_post_install
    ohai "Regenerating CA certificate bundle from keychain, this may take a while..."
    pkgetc.mkpath
    (pkgetc/"cert.pem").atomic_write("ignored")
  end

  def linux_post_install
    cp pkgshare/"cacert.pem", pkgetc/"cert.pem"
  end
end
"#;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator_with_sources(
            vec![formula],
            HashMap::from([("Formula/ca-certificates.rb".to_owned(), source.to_owned())]),
            vec![(sha, tar)],
            counter,
            layout.clone(),
        )?;

        let installed = orchestrator.install(&["ca-certificates"]).await?;

        assert_eq!(installed, vec!["ca-certificates"]);
        assert_eq!(
            std::fs::read_to_string(layout.prefix().join("etc/ca-certificates/cert.pem"))?,
            "bundle"
        );
        assert!(
            layout
                .cellar()
                .join("ca-certificates/1.0/INSTALL_RECEIPT.json")
                .exists()
        );
        assert_installed(&layout, "ca-certificates");
        Ok(())
    }

    #[tokio::test]
    async fn test_install_bootstraps_openssl_cert_symlink_post_install_pattern()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha_ca = "1111111111111111111111111111111111111111111111111111111111111111";
        let sha_ssl = "2222222222222222222222222222222222222222222222222222222222222222";

        let mut ca_formula = make_formula("ca-certificates", "1.0", &[], sha_ca);
        ca_formula.post_install_defined = true;
        let mut ssl_formula = make_formula("openssl@3", "1.0", &["ca-certificates"], sha_ssl);
        ssl_formula.post_install_defined = true;

        let ca_source = r#"
class CaCertificates < Formula
  def post_install
    if OS.mac?
      macos_post_install
    else
      linux_post_install
    end
  end

  def macos_post_install
    pkgetc.mkpath
    (pkgetc/"cert.pem").atomic_write("ignored")
  end

  def linux_post_install
    cp pkgshare/"cacert.pem", pkgetc/"cert.pem"
  end
end
"#;
        let ssl_source = r#"
class OpensslAT3 < Formula
  def openssldir
    etc/"openssl@3"
  end

  def post_install
    rm(openssldir/"cert.pem") if (openssldir/"cert.pem").exist?
    openssldir.install_symlink Formula["ca-certificates"].pkgetc/"cert.pem"
  end
end
"#;

        let orchestrator = make_orchestrator_with_sources(
            vec![ssl_formula, ca_formula],
            HashMap::from([
                (
                    "Formula/ca-certificates.rb".to_owned(),
                    ca_source.to_owned(),
                ),
                ("Formula/openssl@3.rb".to_owned(), ssl_source.to_owned()),
            ]),
            vec![
                (
                    sha_ca,
                    create_bottle_tar_gz(
                        "ca-certificates",
                        "1.0",
                        &[("share/ca-certificates/cacert.pem", b"bundle")],
                    )?,
                ),
                (
                    sha_ssl,
                    create_bottle_tar_gz("openssl@3", "1.0", &[("bin/openssl", b"binary")])?,
                ),
            ],
            Arc::new(AtomicUsize::new(0)),
            layout.clone(),
        )?;

        let installed = orchestrator.install(&["openssl@3"]).await?;

        assert_eq!(installed, vec!["ca-certificates", "openssl@3"]);
        assert!(
            layout
                .prefix()
                .join("etc/ca-certificates/cert.pem")
                .exists()
        );
        let cert_link = layout.prefix().join("etc/openssl@3/cert.pem");
        assert!(cert_link.is_symlink());
        assert_eq!(std::fs::read_to_string(cert_link)?, "bundle");
        Ok(())
    }

    #[tokio::test]
    async fn test_install_rolls_back_post_install_state_when_link_fails()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        std::fs::create_dir_all(layout.prefix().join("bin"))?;
        std::fs::write(layout.prefix().join("bin/tool"), "collision")?;

        let sha = "3434343434343434343434343434343434343434343434343434343434343434";
        let mut formula = make_formula("demo", "1.0", &[], sha);
        formula.post_install_defined = true;

        let source = r#"
class Demo < Formula
  def post_install
    (var/"demo").mkpath
    cp share/"src.txt", var/"demo/copied.txt"
  end
end
"#;

        let orchestrator = make_orchestrator_with_sources(
            vec![formula],
            HashMap::from([("Formula/demo.rb".to_owned(), source.to_owned())]),
            vec![(
                sha,
                create_bottle_tar_gz(
                    "demo",
                    "1.0",
                    &[("share/src.txt", b"payload"), ("bin/tool", b"tool")],
                )?,
            )],
            Arc::new(AtomicUsize::new(0)),
            layout.clone(),
        )?;

        let result = orchestrator.install(&["demo"]).await;

        assert!(matches!(
            result,
            Err(BrewdockError::Cellar(
                brewdock_cellar::CellarError::LinkCollision { .. }
            ))
        ));
        assert!(!layout.cellar().join("demo/1.0").exists());
        assert!(layout.opt_dir().join("demo").symlink_metadata().is_err());
        assert!(!layout.prefix().join("var/demo").exists());
        assert_eq!(
            std::fs::read_to_string(layout.prefix().join("bin/tool"))?,
            "collision"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_source_fallback_install_installs_build_dependency_closure_before_target()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let helper_runtime_sha = "5656565656565656565656565656565656565656565656565656565656565656";
        let build_helper_sha = "6767676767676767676767676767676767676767676767676767676767676767";
        let runtime_dep_sha = "7878787878787878787878787878787878787878787878787878787878787878";
        let source_sha = "8989898989898989898989898989898989898989898989898989898989898989";

        let helper_runtime = make_formula("helper-runtime", "1.0", &[], helper_runtime_sha);
        let build_helper =
            make_formula("build-helper", "1.0", &["helper-runtime"], build_helper_sha);
        let runtime_dep = make_formula("runtime-dep", "1.0", &[], runtime_dep_sha);
        let mut target = make_formula("target", "1.0", &["runtime-dep"], source_sha);
        target.versions.bottle = false;
        target.bottle.stable = None;
        target.build_dependencies = vec!["build-helper".to_owned()];

        let bottles = vec![
            (
                helper_runtime_sha,
                create_bottle_tar_gz(
                    "helper-runtime",
                    "1.0",
                    &[("share/runtime.txt", b"runtime")],
                )?,
            ),
            (
                build_helper_sha,
                create_bottle_tar_gz(
                    "build-helper",
                    "1.0",
                    &[("bin/helper-tool", b"helper-dependency\n")],
                )?,
            ),
            (
                runtime_dep_sha,
                create_bottle_tar_gz("runtime-dep", "1.0", &[("lib/libdep.txt", b"dep")])?,
            ),
            (
                source_sha,
                create_source_tar_gz(
                    "target-1.0",
                    &[(
                        "Makefile",
                        br#"all:
	printf "source-built\n" > target.sh
	cat "$$HOMEBREW_PREFIX/bin/helper-tool" > generated.txt

install:
	mkdir -p "$(PREFIX)/bin"
	cp target.sh "$(PREFIX)/bin/target"
	cp generated.txt "$(PREFIX)/generated.txt"
"#,
                    )],
                )?,
            ),
        ];

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![target, build_helper, helper_runtime, runtime_dep],
            bottles,
            counter,
            layout.clone(),
        )?;

        let installed = orchestrator.install(&["target"]).await?;

        let pos = |name: &str| installed.iter().position(|entry| entry == name);
        assert!(pos("helper-runtime") < pos("build-helper"));
        assert!(pos("build-helper") < pos("target"));
        assert!(pos("runtime-dep") < pos("target"));
        assert_eq!(
            std::fs::read_to_string(layout.cellar().join("target/1.0/generated.txt"))?,
            "helper-dependency\n"
        );
        assert!(layout.prefix().join("bin/target").is_symlink());

        let receipt: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
            layout.cellar().join("target/1.0/INSTALL_RECEIPT.json"),
        )?)?;
        assert_eq!(receipt["built_as_bottle"].as_bool(), Some(false));
        assert_eq!(receipt["poured_from_bottle"].as_bool(), Some(false));
        assert_installed(&layout, "target");
        Ok(())
    }

    #[tokio::test]
    async fn test_source_fallback_rejects_unsupported_requirement_during_planning()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let mut formula = make_formula("a", "1.0", &[], PLAN_SHA);
        formula.versions.bottle = false;
        formula.bottle.stable = None;
        formula.requirements = vec![Requirement::Name("xcode".to_owned())];

        let orchestrator =
            make_orchestrator(vec![formula], vec![], Arc::new(AtomicUsize::new(0)), layout)?;
        let result = orchestrator.plan_install(&["a"]).await;

        assert!(matches!(
            result,
            Err(BrewdockError::SourceBuild(
                SourceBuildError::UnsupportedRequirement(requirement)
            )) if requirement == "xcode"
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_source_fallback_cleans_up_failed_build_without_receipt_or_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let mut formula = make_formula("broken", "1.0", &[], PLAN_SHA);
        formula.versions.bottle = false;
        formula.bottle.stable = None;

        let orchestrator = make_orchestrator(
            vec![formula],
            vec![(
                PLAN_SHA,
                create_source_tar_gz(
                    "broken-1.0",
                    &[(
                        "Makefile",
                        br#"all:
	printf "broken\n" > tool

install:
	mkdir -p "$(PREFIX)/bin"
	cp tool "$(PREFIX)/bin/broken"
	exit 1
"#,
                    )],
                )?,
            )],
            Arc::new(AtomicUsize::new(0)),
            layout.clone(),
        )?;

        let result = orchestrator.install(&["broken"]).await;

        assert!(matches!(
            result,
            Err(BrewdockError::SourceBuild(
                SourceBuildError::CommandFailed { .. }
            ))
        ));
        assert!(!layout.cellar().join("broken/1.0").exists());
        assert!(layout.opt_dir().join("broken").symlink_metadata().is_err());
        assert!(
            !layout
                .cellar()
                .join("broken/1.0/INSTALL_RECEIPT.json")
                .exists()
        );
        assert_not_installed(&layout, "broken");
        Ok(())
    }

    #[tokio::test]
    async fn test_source_fallback_plan_upgrade_reuses_source_method_resolution()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        setup_installed_keg(&layout, "portable", "1.0", true)?;

        let mut formula = make_formula("portable", "2.0", &[], PLAN_SHA);
        formula.versions.bottle = false;
        formula.bottle.stable = None;

        let orchestrator = make_orchestrator(
            vec![formula],
            vec![],
            Arc::new(AtomicUsize::new(0)),
            layout.clone(),
        )?;

        let plan = orchestrator.plan_upgrade(&["portable"]).await?;
        let entry = plan.first().ok_or("expected upgrade plan entry")?;
        assert!(matches!(
            &entry.method,
            InstallMethod::Source(source)
                if source.formula_name == "portable"
                    && source.cellar_path == layout.cellar().join("portable/2.0")
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_source_fallback_upgrade_installs_missing_build_dependencies()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        setup_installed_keg(&layout, "target", "1.0", true)?;

        let helper_sha = "9090909090909090909090909090909090909090909090909090909090909090";
        let source_sha = "9191919191919191919191919191919191919191919191919191919191919191";

        let helper = make_formula("build-helper", "1.0", &[], helper_sha);
        let mut target = make_formula("target", "2.0", &[], source_sha);
        target.versions.bottle = false;
        target.bottle.stable = None;
        target.build_dependencies = vec!["build-helper".to_owned()];

        let orchestrator = make_orchestrator(
            vec![target, helper],
            vec![
                (
                    helper_sha,
                    create_bottle_tar_gz(
                        "build-helper",
                        "1.0",
                        &[("bin/helper-tool", b"helper-upgrade\n")],
                    )?,
                ),
                (
                    source_sha,
                    create_source_tar_gz(
                        "target-2.0",
                        &[(
                            "Makefile",
                            br#"all:
	cat "$$HOMEBREW_PREFIX/bin/helper-tool" > generated.txt
	printf "target\n" > target

install:
	mkdir -p "$(PREFIX)/bin"
	cp target "$(PREFIX)/bin/target"
	cp generated.txt "$(PREFIX)/generated.txt"
"#,
                        )],
                    )?,
                ),
            ],
            Arc::new(AtomicUsize::new(0)),
            layout.clone(),
        )?;

        let upgraded = orchestrator.upgrade(&["target"]).await?;

        assert_eq!(upgraded, vec!["target"]);
        assert_eq!(
            std::fs::read_to_string(layout.cellar().join("target/2.0/generated.txt"))?,
            "helper-upgrade\n"
        );
        assert_installed(&layout, "build-helper");
        let target_keg = find_installed_keg("target", &layout.cellar(), &layout.opt_dir())?
            .ok_or("expected upgraded target record")?;
        assert_eq!(target_keg.pkg_version, "2.0");
        Ok(())
    }

    // --- Upgrade tests ---

    #[tokio::test]
    async fn test_upgrade_unlinks_old_and_installs_new() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        // Set up old installation manually.
        let old_keg = layout.cellar().join("a/1.0");
        std::fs::create_dir_all(old_keg.join("bin"))?;
        std::fs::write(old_keg.join("bin/a_tool"), "old_version")?;
        link(&old_keg, layout.prefix())?;

        // Verify old symlink resolves to old content.
        assert_eq!(
            std::fs::read_to_string(layout.prefix().join("bin/a_tool"))?,
            "old_version"
        );

        // Write receipt for the old version so filesystem state discovery finds it.
        write_receipt(
            &old_keg,
            &InstallReceipt::for_bottle(
                InstallReason::OnRequest,
                Some(1_000.0),
                Vec::new(),
                ReceiptSource {
                    path: String::new(),
                    tap: "homebrew/core".to_owned(),
                    spec: "stable".to_owned(),
                    versions: ReceiptSourceVersions {
                        stable: "1.0".to_owned(),
                        head: None,
                        version_scheme: 0,
                    },
                },
            ),
        )?;
        // Create opt symlink pointing to old keg.
        std::fs::create_dir_all(layout.opt_dir())?;
        atomic_symlink_replace(&old_keg, &layout.opt_dir().join("a"))?;

        // Mock repo returns v2.0.
        let sha = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let formula = make_formula("a", "2.0", &[], sha);
        let tar = create_bottle_tar_gz("a", "2.0", &[("bin/a_tool", b"new_version")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula],
            vec![(sha, tar)],
            counter.clone(),
            layout.clone(),
        )?;

        let upgraded = orchestrator.upgrade(&["a"]).await?;

        assert_eq!(upgraded, vec!["a"]);

        // New keg exists with new content.
        assert_eq!(
            std::fs::read_to_string(layout.cellar().join("a/2.0/bin/a_tool"))?,
            "new_version"
        );

        // Symlink points to new version.
        assert!(layout.prefix().join("bin/a_tool").is_symlink());
        assert_eq!(
            std::fs::read_to_string(layout.prefix().join("bin/a_tool"))?,
            "new_version"
        );

        // Filesystem state updated, preserving installed_on_request status.
        let keg =
            find_installed_keg("a", &layout.cellar(), &layout.opt_dir())?.ok_or("expected keg")?;
        assert_eq!(keg.pkg_version, "2.0");
        assert!(keg.installed_on_request);
        Ok(())
    }

    #[tokio::test]
    async fn test_upgrade_skips_current_version() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        // Pre-populate filesystem state with v1.0 (same as repo).
        setup_installed_keg(&layout, "a", "1.0", true)?;

        let formula = make_formula(
            "a",
            "1.0",
            &[],
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        );
        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula], vec![], counter.clone(), layout.clone())?;

        let upgraded = orchestrator.upgrade(&["a"]).await?;

        // Nothing upgraded.
        assert!(upgraded.is_empty());
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        Ok(())
    }

    // --- Update tests ---

    #[tokio::test]
    async fn test_update_caches_formula_index() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let formula_a = make_formula(
            "a",
            "1.0",
            &[],
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let formula_b = make_formula(
            "b",
            "2.0",
            &[],
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        );

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula_a, formula_b], vec![], counter, layout.clone())?;

        let count = orchestrator.update().await?;
        assert_eq!(count, 2);

        let cache_path = layout.cache_dir().join("formula.json");
        assert!(cache_path.exists());

        let data = std::fs::read_to_string(&cache_path)?;
        let cached: Vec<Formula> = serde_json::from_str(&data)?;
        assert_eq!(cached.len(), 2);

        let names: std::collections::HashSet<String> = cached.into_iter().map(|f| f.name).collect();
        assert!(names.contains("a"));
        assert!(names.contains("b"));
        Ok(())
    }

    /// Sets the cellar type on the host bottle of a formula.
    fn set_bottle_cellar(formula: &mut Formula, cellar: CellarType) {
        if let Some(ref mut stable) = formula.bottle.stable
            && let Some(file) = stable.files.get_mut(HOST_TAG)
        {
            file.cellar = cellar;
        }
    }

    #[tokio::test]
    async fn test_resolve_incompatible_cellar_bottle_without_source_fallback_returns_error()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let mut formula = make_formula("a", "1.0", &[], PLAN_SHA);
        // Set a cellar path that won't match the tempdir-based layout.
        set_bottle_cellar(
            &mut formula,
            CellarType::Path("/usr/local/Cellar".to_owned()),
        );
        // Remove source URL so there is no fallback.
        formula.urls.stable = None;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout)?;

        let result = orchestrator.plan_install(&["a"]).await;

        assert!(matches!(
            result,
            Err(BrewdockError::Formula(FormulaError::Unsupported {
                reason: UnsupportedReason::IncompatibleCellar(_),
                ..
            }))
        ));
        Ok(())
    }

    #[tokio::test]
    async fn test_resolve_incompatible_cellar_bottle_falls_back_to_source()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let mut formula = make_formula("a", "1.0", &[], PLAN_SHA);
        set_bottle_cellar(
            &mut formula,
            CellarType::Path("/usr/local/Cellar".to_owned()),
        );

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout)?;

        let plan = orchestrator.plan_install(&["a"]).await?;
        let entry = plan.first().ok_or("expected install plan entry")?;
        assert!(matches!(&entry.method, InstallMethod::Source(_)));
        Ok(())
    }

    #[tokio::test]
    async fn test_install_any_skip_relocation_bottle_succeeds()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha = "abababababababababababababababababababababababababababababababab";
        let mut formula = make_formula("skip-reloc", "1.0", &[], sha);
        set_bottle_cellar(&mut formula, CellarType::AnySkipRelocation);

        let tar =
            create_bottle_tar_gz("skip-reloc", "1.0", &[("bin/tool", b"#!/bin/sh\necho ok")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula],
            vec![(sha, tar)],
            counter.clone(),
            layout.clone(),
        )?;

        let installed = orchestrator.install(&["skip-reloc"]).await?;

        assert_eq!(installed, vec!["skip-reloc"]);
        assert!(layout.cellar().join("skip-reloc/1.0/bin/tool").exists());
        assert!(layout.opt_dir().join("skip-reloc").is_symlink());
        Ok(())
    }

    #[tokio::test]
    async fn test_any_skip_relocation_bottle_relocates_text_placeholders()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
        let mut formula = make_formula("pytools", "3.0", &[], sha);
        set_bottle_cellar(&mut formula, CellarType::AnySkipRelocation);

        let shebang = b"#!@@HOMEBREW_PREFIX@@/bin/python3\nimport sys\n";
        let pth = b"@@HOMEBREW_CELLAR@@/pytools/3.0/lib/python3\n";
        let tar = create_bottle_tar_gz(
            "pytools",
            "3.0",
            &[("bin/tool", shebang), ("lib/python3/site.pth", pth)],
        )?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula],
            vec![(sha, tar)],
            counter.clone(),
            layout.clone(),
        )?;

        orchestrator.install(&["pytools"]).await?;

        let installed_shebang =
            std::fs::read_to_string(layout.cellar().join("pytools/3.0/bin/tool"))?;
        assert_eq!(
            installed_shebang,
            format!("#!{}/bin/python3\nimport sys\n", layout.prefix().display(),),
            "text placeholders in shebang must be replaced for any_skip_relocation bottles",
        );

        let installed_pth =
            std::fs::read_to_string(layout.cellar().join("pytools/3.0/lib/python3/site.pth"))?;
        assert_eq!(
            installed_pth,
            format!(
                "{}/Cellar/pytools/3.0/lib/python3\n",
                layout.prefix().display()
            ),
            "text placeholders in .pth must be replaced for any_skip_relocation bottles",
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_upgrade_restores_old_links_when_new_payload_download_fails()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());
        setup_installed_keg(&layout, "demo", "1.0", true)?;
        let old_keg = layout.cellar().join("demo/1.0");
        std::fs::create_dir_all(old_keg.join("bin"))?;
        std::fs::create_dir_all(layout.prefix().join("bin"))?;
        std::fs::write(old_keg.join("bin/tool"), "#!/bin/sh\necho old")?;
        std::os::unix::fs::symlink(
            "../Cellar/demo/1.0/bin/tool",
            layout.prefix().join("bin/tool"),
        )?;

        let sha = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let formula = make_formula("demo", "2.0", &[], sha);
        let download_count = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula],
            vec![],
            download_count.clone(),
            layout.clone(),
        )?;

        let result = orchestrator.upgrade(&["demo"]).await;

        assert!(matches!(
            result,
            Err(BrewdockError::Bottle(_) | BrewdockError::Cellar(_) | BrewdockError::Io(_))
        ));
        assert!(layout.cellar().join("demo/1.0/bin/tool").exists());
        let opt_link = layout.opt_dir().join("demo");
        assert!(
            opt_link.is_symlink(),
            "rollback should restore the opt/demo symlink"
        );
        assert_eq!(
            std::fs::read_link(&opt_link)?,
            layout.cellar().join("demo/1.0")
        );
        assert!(layout.prefix().join("bin/tool").is_symlink());
        assert_eq!(download_count.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn test_install_with_empty_input_is_noop() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());
        let orchestrator = make_orchestrator(
            vec![],
            vec![],
            Arc::new(AtomicUsize::new(0)),
            layout.clone(),
        )?;

        let installed = orchestrator.install(&[]).await?;

        assert!(installed.is_empty());
        assert!(!layout.cellar().exists());
        assert!(!layout.opt_dir().exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_source_archive_traversal_does_not_escape_cache_root()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());
        let sha = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        let mut formula = make_formula("evil", "1.0", &[], sha);
        formula.bottle.stable = None;
        let archive = create_source_tar_gz_with_raw_paths(&[
            (b"evil-1.0/README.md", b"legit source tree"),
            (b"../../escape.txt", b"escaped"),
        ])?;
        let orchestrator = make_orchestrator(
            vec![formula],
            vec![(sha, archive)],
            Arc::new(AtomicUsize::new(0)),
            layout.clone(),
        )?;

        let result = orchestrator.install(&["evil"]).await;

        assert!(
            result.is_err(),
            "malformed source archive should fail closed"
        );
        assert!(
            !layout.cache_dir().join("sources/escape.txt").exists(),
            "path traversal must not escape the temporary source root"
        );
        assert!(!layout.cellar().join("evil/1.0").exists());
        assert!(!layout.opt_dir().join("evil").exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_install_blocks_while_lock_is_held() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());
        let sha = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        let formula = make_formula("locky", "1.0", &[], sha);
        let tar = create_simple_bottle("locky", "1.0")?;
        let download_count = Arc::new(AtomicUsize::new(0));
        let orchestrator = Arc::new(make_orchestrator(
            vec![formula],
            vec![(sha, tar)],
            download_count.clone(),
            layout.clone(),
        )?);

        let lock_path = layout.lock_dir().join("brewdock.lock");
        let lock = FileLock::acquire(&lock_path)?;

        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let handle = {
            let orchestrator = Arc::clone(&orchestrator);
            thread::spawn(move || -> Result<(), String> {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .map_err(|error| error.to_string())?;
                started_tx.send(()).map_err(|error| error.to_string())?;
                runtime
                    .block_on(async { orchestrator.install(&["locky"]).await })
                    .map_err(|error| error.to_string())?;
                Ok(())
            })
        };

        started_rx
            .recv_timeout(Duration::from_secs(1))
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        thread::sleep(Duration::from_millis(100));
        assert_eq!(
            download_count.load(Ordering::SeqCst),
            0,
            "install must not start downloading while the lock is held"
        );

        drop(lock);

        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(std::io::Error::other(error).into()),
            Err(_) => return Err(std::io::Error::other("install thread panicked").into()),
        }

        assert_eq!(download_count.load(Ordering::SeqCst), 1);
        assert!(layout.cellar().join("locky/1.0/bin/tool").exists());
        Ok(())
    }

    // --- Parallel install tests ---

    #[tokio::test]
    async fn test_install_independent_formulae_all_installed()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let sha_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let sha_c = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let formula_a = make_formula("alpha", "1.0", &[], sha_a);
        let formula_b = make_formula("bravo", "2.0", &[], sha_b);
        let formula_c = make_formula("charlie", "3.0", &[], sha_c);

        let tar_a =
            create_bottle_tar_gz("alpha", "1.0", &[("bin/alpha_tool", b"#!/bin/sh\necho a")])?;
        let tar_b =
            create_bottle_tar_gz("bravo", "2.0", &[("bin/bravo_tool", b"#!/bin/sh\necho b")])?;
        let tar_c = create_bottle_tar_gz(
            "charlie",
            "3.0",
            &[("bin/charlie_tool", b"#!/bin/sh\necho c")],
        )?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula_a, formula_b, formula_c],
            vec![(sha_a, tar_a), (sha_b, tar_b), (sha_c, tar_c)],
            counter.clone(),
            layout.clone(),
        )?;

        let installed = orchestrator.install(&["alpha", "bravo", "charlie"]).await?;

        assert_eq!(installed.len(), 3);
        assert_installed(&layout, "alpha");
        assert_installed(&layout, "bravo");
        assert_installed(&layout, "charlie");
        assert_eq!(counter.load(Ordering::SeqCst), 3);
        Ok(())
    }

    #[tokio::test]
    async fn test_install_finalize_failure_preserves_completed_installs()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let sha_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let sha_c = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        // alpha installs bin/tool, charlie also installs bin/tool → link collision on charlie
        let formula_a = make_formula("alpha", "1.0", &[], sha_a);
        let formula_b = make_formula("bravo", "2.0", &[], sha_b);
        let formula_c = make_formula("charlie", "3.0", &[], sha_c);

        let tar_a = create_bottle_tar_gz("alpha", "1.0", &[("bin/tool", b"#!/bin/sh\necho a")])?;
        let tar_b =
            create_bottle_tar_gz("bravo", "2.0", &[("bin/bravo_tool", b"#!/bin/sh\necho b")])?;
        // charlie collides with alpha on bin/tool
        let tar_c = create_bottle_tar_gz("charlie", "3.0", &[("bin/tool", b"#!/bin/sh\necho c")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula_a, formula_b, formula_c],
            vec![(sha_a, tar_a), (sha_b, tar_b), (sha_c, tar_c)],
            counter.clone(),
            layout.clone(),
        )?;

        let result = orchestrator.install(&["alpha", "bravo", "charlie"]).await;

        // charlie fails due to link collision
        assert!(result.is_err());

        // alpha and bravo were finalized before charlie, so they remain installed
        assert_installed(&layout, "alpha");
        assert_installed(&layout, "bravo");
        // charlie was rolled back
        assert_not_installed(&layout, "charlie");
        Ok(())
    }

    #[tokio::test]
    async fn test_install_skips_download_when_blob_exists_in_store()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let formula_a = make_formula("a", "1.0", &[], sha_a);
        let tar_a = create_simple_bottle("a", "1.0")?;

        // Pre-populate blob store so download should be skipped.
        let blob_store = BlobStore::new(&layout.blob_dir());
        blob_store.put(sha_a, &tar_a)?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula_a],
            vec![(sha_a, tar_a)],
            counter.clone(),
            layout.clone(),
        )?;

        let installed = orchestrator.install(&["a"]).await?;

        assert_eq!(installed, vec!["a"]);
        assert_installed(&layout, "a");
        // Download should have been skipped because blob already existed.
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "download should be skipped when blob exists in store"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_install_skips_extract_when_extract_dir_exists()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let formula_a = make_formula("a", "1.0", &[], sha_a);
        let tar_a = create_bottle_tar_gz("a", "1.0", &[("bin/tool", b"#!/bin/sh\necho ok\n")])?;

        // Pre-populate blob store and extract dir so both download and extract are skipped.
        let blob_store = BlobStore::new(&layout.blob_dir());
        blob_store.put(sha_a, &tar_a)?;
        let blob_path = blob_store.blob_path(sha_a)?;

        let extract_dir = layout.store_dir().join(sha_a);
        extract_tar_gz(&blob_path, &extract_dir)?;
        // Extract dir now has a/1.0/bin/tool

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula_a],
            vec![(sha_a, tar_a)],
            counter.clone(),
            layout.clone(),
        )?;

        let installed = orchestrator.install(&["a"]).await?;

        assert_eq!(installed, vec!["a"]);
        assert_installed(&layout, "a");
        // Download and extract were both skipped.
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "download should be skipped when blob exists"
        );
        // Formula should still be correctly installed from cached extract.
        assert!(layout.cellar().join("a/1.0/bin/tool").exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_install_warm_path_with_multiple_formulae()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let sha_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let formula_a = make_formula("a", "1.0", &[], sha_a);
        let formula_b = make_formula("b", "2.0", &[], sha_b);

        let tar_a = create_bottle_tar_gz("a", "1.0", &[("bin/a_tool", b"#!/bin/sh\necho a")])?;
        let tar_b = create_bottle_tar_gz("b", "2.0", &[("bin/b_tool", b"#!/bin/sh\necho b")])?;

        // Pre-populate blob store for both.
        let blob_store = BlobStore::new(&layout.blob_dir());
        blob_store.put(sha_a, &tar_a)?;
        blob_store.put(sha_b, &tar_b)?;

        // Pre-populate extract dirs for both.
        let blob_a = blob_store.blob_path(sha_a)?;
        let blob_b = blob_store.blob_path(sha_b)?;
        extract_tar_gz(&blob_a, &layout.store_dir().join(sha_a))?;
        extract_tar_gz(&blob_b, &layout.store_dir().join(sha_b))?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula_a, formula_b],
            vec![(sha_a, tar_a), (sha_b, tar_b)],
            counter.clone(),
            layout.clone(),
        )?;

        let installed = orchestrator.install(&["a", "b"]).await?;

        assert_eq!(installed.len(), 2);
        assert_installed(&layout, "a");
        assert_installed(&layout, "b");
        // Zero downloads — all warm cache hits.
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn test_install_download_failure_prevents_prefix_mutation()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let sha_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let sha_c = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let formula_a = make_formula("alpha", "1.0", &[], sha_a);
        let formula_b = make_formula("bravo", "2.0", &[], sha_b);
        let formula_c = make_formula("charlie", "3.0", &[], sha_c);

        let tar_a =
            create_bottle_tar_gz("alpha", "1.0", &[("bin/alpha_tool", b"#!/bin/sh\necho a")])?;
        let tar_b =
            create_bottle_tar_gz("bravo", "2.0", &[("bin/bravo_tool", b"#!/bin/sh\necho b")])?;
        // charlie has NO bottle data → download fails

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula_a, formula_b, formula_c],
            vec![(sha_a, tar_a), (sha_b, tar_b)],
            counter.clone(),
            layout.clone(),
        )?;

        let result = orchestrator.install(&["alpha", "bravo", "charlie"]).await;

        assert!(result.is_err());
        // With prefetch-first architecture: no prefix mutation should have occurred
        // because all downloads happen before any finalization
        assert_not_installed(&layout, "alpha");
        assert_not_installed(&layout, "bravo");
        assert_not_installed(&layout, "charlie");
        Ok(())
    }

    // --- Metadata cache tests ---

    /// Mock that tracks per-formula fetch calls to verify cache usage.
    struct CountingMockRepo {
        formulae: HashMap<String, Formula>,
        formula_call_count: Arc<AtomicUsize>,
    }

    impl CountingMockRepo {
        fn new(list: Vec<Formula>, counter: Arc<AtomicUsize>) -> Self {
            let formulae = list.into_iter().map(|f| (f.name.clone(), f)).collect();
            Self {
                formulae,
                formula_call_count: counter,
            }
        }
    }

    impl FormulaRepository for CountingMockRepo {
        async fn formula(&self, name: &str) -> Result<Formula, FormulaError> {
            self.formula_call_count.fetch_add(1, Ordering::SeqCst);
            self.formulae
                .get(name)
                .cloned()
                .ok_or_else(|| FormulaError::NotFound {
                    name: FormulaName::from(name),
                })
        }

        async fn all_formulae(&self) -> Result<Vec<Formula>, FormulaError> {
            Ok(self.formulae.values().cloned().collect())
        }

        async fn ruby_source(&self, _ruby_source_path: &str) -> Result<String, FormulaError> {
            Err(FormulaError::NotFound {
                name: FormulaName::from("unsupported"),
            })
        }
    }

    #[tokio::test]
    async fn test_update_writes_metadata_and_formulae_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());
        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![make_formula("jq", "1.8.1", &["oniguruma"], PLAN_SHA)],
            vec![],
            counter,
            layout.clone(),
        )?;

        let count = orchestrator.update().await?;

        assert_eq!(count, 1);
        assert!(layout.cache_dir().join("formula.json").exists());
        assert!(layout.cache_dir().join("formula-meta.json").exists());

        let store = brewdock_formula::MetadataStore::new(layout.cache_dir());
        let meta = store.load_metadata()?.ok_or("metadata should exist")?;
        assert!(meta.fetched_at > 0);
        Ok(())
    }

    #[tokio::test]
    async fn test_install_plan_uses_cached_metadata() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        // Pre-populate the disk cache using MetadataStore
        let store = brewdock_formula::MetadataStore::new(layout.cache_dir());
        store.save_formulae(&[
            make_formula("jq", "1.8.1", &["oniguruma"], PLAN_SHA),
            make_formula("oniguruma", "6.9.9", &[], PLAN_SHA),
        ])?;

        // Create orchestrator with a counting mock
        let formula_calls = Arc::new(AtomicUsize::new(0));
        let host_tag: HostTag = HOST_TAG.parse()?;
        let repo = CountingMockRepo::new(
            vec![
                make_formula("jq", "1.8.1", &["oniguruma"], PLAN_SHA),
                make_formula("oniguruma", "6.9.9", &[], PLAN_SHA),
            ],
            Arc::clone(&formula_calls),
        );
        let download_count = Arc::new(AtomicUsize::new(0));
        let downloader = MockDownloader::new(vec![], download_count);
        let orchestrator = Orchestrator::new(repo, downloader, layout, host_tag);

        let plan = orchestrator.plan_install(&["jq"]).await?;

        // Cache had both jq and oniguruma, so no individual formula() calls
        assert_eq!(
            formula_calls.load(Ordering::SeqCst),
            0,
            "should use disk cache, not network"
        );
        assert_eq!(plan.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn test_install_plan_falls_back_to_network_on_cache_miss()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        // Pre-populate cache with only jq (missing oniguruma)
        let store = brewdock_formula::MetadataStore::new(layout.cache_dir());
        store.save_formulae(&[make_formula("jq", "1.8.1", &["oniguruma"], PLAN_SHA)])?;

        // Create orchestrator with counting mock that has both
        let formula_calls = Arc::new(AtomicUsize::new(0));
        let host_tag: HostTag = HOST_TAG.parse()?;
        let repo = CountingMockRepo::new(
            vec![
                make_formula("jq", "1.8.1", &["oniguruma"], PLAN_SHA),
                make_formula("oniguruma", "6.9.9", &[], PLAN_SHA),
            ],
            Arc::clone(&formula_calls),
        );
        let download_count = Arc::new(AtomicUsize::new(0));
        let downloader = MockDownloader::new(vec![], download_count);
        let orchestrator = Orchestrator::new(repo, downloader, layout, host_tag);

        let plan = orchestrator.plan_install(&["jq"]).await?;

        // jq from cache, oniguruma from network (1 call)
        assert_eq!(
            formula_calls.load(Ordering::SeqCst),
            1,
            "should fetch only the missing dependency from network"
        );
        assert_eq!(plan.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn test_install_plan_works_without_cache() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        // No disk cache populated
        let formula_calls = Arc::new(AtomicUsize::new(0));
        let host_tag: HostTag = HOST_TAG.parse()?;
        let repo = CountingMockRepo::new(
            vec![
                make_formula("jq", "1.8.1", &["oniguruma"], PLAN_SHA),
                make_formula("oniguruma", "6.9.9", &[], PLAN_SHA),
            ],
            Arc::clone(&formula_calls),
        );
        let download_count = Arc::new(AtomicUsize::new(0));
        let downloader = MockDownloader::new(vec![], download_count);
        let orchestrator = Orchestrator::new(repo, downloader, layout, host_tag);

        let plan = orchestrator.plan_install(&["jq"]).await?;

        // No cache, all formulae fetched from network
        assert_eq!(
            formula_calls.load(Ordering::SeqCst),
            2,
            "should fetch all from network when no cache"
        );
        assert_eq!(plan.len(), 2);
        Ok(())
    }
}
