use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    path::{Path, PathBuf},
};

use brewdock_bottle::{BlobStore, BottleDownloader};
use brewdock_cellar::{
    InstalledKeg, PlatformContext, discover_installed_kegs, find_installed_keg, link, unlink,
};
use brewdock_formula::{
    FetchOutcome, Formula, FormulaCache, FormulaError, FormulaName, FormulaRepository,
    IndexMetadata, MetadataStore, PkgVersion, SelectedBottle, UnsupportedReason, select_bottle,
};
use tracing::Instrument;

use crate::{
    BrewdockError, HostTag, Layout,
    finalize::{build_receipt, build_receipt_deps, build_receipt_source, refresh_opt_link},
    lock::FileLock,
    platform::detect_platform_context,
    progress::{NoopProgressSink, ProgressEvent, SharedProgressSink},
    source_build::build_source_plan,
};

mod execution;
mod planning;
mod query;

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

/// An outdated formula with current and latest version information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutdatedEntry {
    /// Formula name.
    pub name: String,
    /// Currently installed version.
    pub current_version: String,
    /// Latest available version.
    pub latest_version: String,
}

/// Formula information for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaInfo {
    /// Formula name.
    pub name: String,
    /// Stable version.
    pub version: String,
    /// Short description.
    pub desc: Option<String>,
    /// Homepage URL.
    pub homepage: Option<String>,
    /// License identifier.
    pub license: Option<String>,
    /// Whether the formula is keg-only.
    pub keg_only: bool,
    /// Runtime dependencies.
    pub dependencies: Vec<String>,
    /// Whether a bottle is available for the host tag.
    pub bottle_available: bool,
    /// Currently installed version, if any.
    pub installed_version: Option<String>,
}

/// Result of a cleanup operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupResult {
    /// Number of blob files removed.
    pub blobs_removed: u64,
    /// Number of extracted store directories removed.
    pub stores_removed: u64,
    /// Total bytes freed.
    pub bytes_freed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutionPlan<'a> {
    entries: Vec<ExecutionPlanEntry<'a>>,
    acquire_concurrency: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExecutionPlanEntry<'a> {
    formula: &'a Formula,
    method: InstallMethod,
    finalize: FinalizeStep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FinalizeStep {
    FinalizeBottle,
    BuildFromSource,
}

impl<'a> ExecutionPlanEntry<'a> {
    const fn new(formula: &'a Formula, method: InstallMethod) -> Self {
        let finalize = match &method {
            InstallMethod::Bottle(_) => FinalizeStep::FinalizeBottle,
            InstallMethod::Source(_) => FinalizeStep::BuildFromSource,
        };
        Self {
            formula,
            method,
            finalize,
        }
    }
}

/// A diagnostic finding from the doctor command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticEntry {
    /// Category of the finding.
    pub category: DiagnosticCategory,
    /// Human-readable message.
    pub message: String,
}

/// Category of a diagnostic finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticCategory {
    /// Everything is fine.
    Ok,
    /// Something that might cause issues.
    Warning,
}

impl std::fmt::Display for DiagnosticCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ok => f.write_str("ok"),
            Self::Warning => f.write_str("warning"),
        }
    }
}

/// Orchestrates formula installation and upgrade operations.
///
/// Generic over `R` (formula repository) and `D` (bottle downloader) for
/// testability via mock implementations.
///
/// The install pipeline is driven by an explicit execution plan that fixes
/// the `acquire -> finalize` boundary per formula. `acquire` merges
/// download and materialization into a single streaming stage with bounded
/// concurrency. `finalize` remains serialized in topological order to
/// preserve Homebrew-visible state transitions.
pub struct Orchestrator<R, D> {
    repo: R,
    downloader: D,
    layout: Layout,
    host_tag: HostTag,
    metadata_store: MetadataStore,
    progress_sink: SharedProgressSink,
    platform: PlatformContext,
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

/// Bundles a resolved install method with its acquired payload.
struct AcquiredFormula {
    method: InstallMethod,
    finalize: FinalizeStep,
    acquired: AcquiredPayload,
}

/// Bundles method and keg path for finalization.
struct FinalizeContext<'a> {
    method: &'a InstallMethod,
    keg_path: &'a Path,
}

/// Result of the acquire stage for a single formula.
///
/// Bottles are downloaded, extracted, materialized, and relocated during
/// the concurrent acquire stage. Source builds defer the actual build to
/// the serial finalize stage because they may depend on other formulae
/// being linked first.
enum AcquiredPayload {
    /// A bottle was materialized into its keg and relocated.
    Bottle {
        /// Target keg path (`Cellar/name/version`).
        keg_path: PathBuf,
    },
    /// Source build is deferred to the finalize stage.
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
    /// Maximum age in seconds before the metadata cache is considered stale.
    ///
    /// Matches Homebrew's default staleness window (7 days).
    const CACHE_MAX_AGE_SECS: u64 = 7 * 24 * 3600;
    const MAX_ACQUIRE_CONCURRENCY: usize = 4;

    /// Creates a new orchestrator.
    #[must_use]
    pub fn new(repo: R, downloader: D, layout: Layout, host_tag: HostTag) -> Self {
        Self::with_progress_sink(
            repo,
            downloader,
            layout,
            host_tag,
            std::sync::Arc::new(NoopProgressSink),
        )
    }

    /// Creates a new orchestrator with a user-facing progress sink.
    #[must_use]
    pub fn with_progress_sink(
        repo: R,
        downloader: D,
        layout: Layout,
        host_tag: HostTag,
        progress_sink: SharedProgressSink,
    ) -> Self {
        let metadata_store = MetadataStore::new(layout.cache_dir());
        Self {
            repo,
            downloader,
            layout,
            host_tag,
            metadata_store,
            progress_sink,
            platform: detect_platform(),
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
        self.instrument_operation("install-plan", names, async {
            let label = request_label(names);
            let _lock = self.acquire_lock()?;
            let (to_install, cache) = self
                .instrument_async_phase(
                    "install-plan",
                    "resolve-install-list",
                    &label,
                    self.resolve_install_list(names),
                )
                .await?;
            self.instrument_phase("install-plan", "resolve-methods", &label, || {
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
    /// The pipeline is split into two stages:
    /// 1. **Acquire**: download, verify, store, extract, and (for bottles)
    ///    materialize and relocate all payloads concurrently. Each keg is
    ///    independent so no serialization is needed. Source builds defer the
    ///    actual build to finalize. Blob store and extract dir hits skip
    ///    download and extraction respectively (warm-path).
    /// 2. **Finalize**: post-install, link, and write receipts in topological
    ///    order. Source builds run here because they may need linked dependencies.
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if any step fails. Unsupported formulae are
    /// rejected before any download begins.
    pub async fn install(&self, names: &[&str]) -> Result<Vec<String>, BrewdockError> {
        self.instrument_operation("install", names, async {
            let _lock = self.acquire_lock()?;
            let label = request_label(names);
            let (to_install, cache) = self
                .instrument_async_phase(
                    "install",
                    "resolve-install-list",
                    &label,
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
            self.execute_install_plan(&label, &to_install, &cache, &install_context)
                .await?;

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
        self.instrument_operation("update", &[], async {
            // Only use ETag for conditional fetch when both metadata and
            // formulae are present in the store. If the store is empty
            // (e.g., new database), force a full re-fetch to restore
            // integrity rather than accepting a 304 with no local data.
            let existing_meta = match self.metadata_store.load_metadata() {
                Ok(meta) => meta,
                Err(err) => {
                    self.emit_warning(
                        "update",
                        "formula-index",
                        "failed to read metadata cache, forcing full re-fetch",
                    );
                    tracing::warn!(%err, "failed to read metadata cache, forcing full re-fetch");
                    None
                }
            };
            let existing_etag = existing_meta
                .as_ref()
                .and_then(|m| m.etag.as_deref())
                .filter(|_| existing_meta.as_ref().is_some_and(|m| m.formula_count > 0));

            let outcome = self
                .instrument_async_phase(
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
        self.instrument_operation("upgrade-plan", names, async {
            let _lock = self.acquire_lock()?;
            let candidates = self
                .instrument_async_phase(
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
        self.instrument_operation("upgrade", names, async {
            let _lock = self.acquire_lock()?;
            let candidates = self
                .instrument_async_phase(
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
                    self.instrument_phase("upgrade", "unlink-old-keg", &candidate.name, || {
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

                let install_result = self
                    .instrument_async_phase(
                        "upgrade",
                        "install-target-version",
                        &candidate.name,
                        self.run_upgrade_install(&candidate, &install_context),
                    )
                    .await;

                if let Err(error) = install_result {
                    self.emit_warning(
                        "upgrade",
                        &candidate.name,
                        "upgrade failed, restoring previous version",
                    );
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

    /// Ensures the metadata cache is populated and fresh.
    ///
    /// If the cache is empty or older than [`Self::CACHE_MAX_AGE_SECS`],
    /// fetches the formula index from the network (equivalent to `bd update`).
    async fn ensure_fresh_cache(&self) -> Result<(), BrewdockError> {
        let needs_refresh = match self.metadata_store.load_metadata() {
            Ok(Some(meta)) if meta.formula_count > 0 => {
                let age = unix_now().as_secs().saturating_sub(meta.fetched_at);
                age > Self::CACHE_MAX_AGE_SECS
            }
            _ => true,
        };
        if needs_refresh {
            self.update().await?;
        }
        Ok(())
    }

    /// Persists the formula index and freshness metadata to the `SQLite` store.
    fn persist_formula_index(
        &self,
        formulae: &[Formula],
        etag: Option<String>,
    ) -> Result<(), BrewdockError> {
        self.instrument_phase(
            "update",
            "persist-formula-index",
            "formula-index",
            || -> Result<(), BrewdockError> {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                Ok(self.metadata_store.save_index(
                    formulae,
                    &IndexMetadata {
                        etag,
                        fetched_at: now,
                        formula_count: formulae.len(),
                    },
                )?)
            },
        )
    }

    /// Acquires the brewdock file lock.
    fn acquire_lock(&self) -> Result<FileLock, std::io::Error> {
        FileLock::acquire(&self.layout.lock_dir().join("brewdock.lock"))
    }

    /// Resolves a single formula from the local `SQLite` cache, falling back
    /// to the network if not found locally.
    async fn resolve_formula(&self, name: &str) -> Result<Formula, BrewdockError> {
        match self.metadata_store.load_formula(name) {
            Ok(Some(formula)) => return Ok(formula),
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(
                    %err,
                    name,
                    "failed to read formula from cache, falling back to network"
                );
            }
        }
        Ok(self.repo.formula(name).await?)
    }

    /// Fetches formulae and all transitive dependencies.
    ///
    /// Checks the on-disk metadata cache (per-formula `SQLite` lookup) first
    /// and falls back to the network for any formula not found locally.
    async fn fetch_with_deps(&self, names: &[&str]) -> Result<FormulaCache, BrewdockError> {
        let mut cache = FormulaCache::new();
        let mut queue: VecDeque<String> = names.iter().map(|name| (*name).to_owned()).collect();

        while let Some(name) = queue.pop_front() {
            if cache.get(&name).is_some() {
                continue;
            }
            let formula = self.resolve_formula(&name).await?;
            for dep in self.plan_dependencies(&formula)? {
                if cache.get(&dep).is_none() {
                    queue.push_back(dep);
                }
            }
            cache.insert(formula);
        }

        Ok(cache)
    }

    fn resolve_install_method(&self, formula: &Formula) -> Result<InstallMethod, BrewdockError> {
        let selected = select_bottle(formula, self.host_tag.as_str());

        if let Some(bottle) = selected {
            if bottle.cellar.is_compatible(&self.layout.cellar()) {
                return Ok(InstallMethod::Bottle(bottle));
            }
            self.emit_warning(
                "install-resolution",
                &formula.name,
                "bottle requires incompatible cellar path, trying source fallback",
            );
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
            &self.host_tag,
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

    fn build_execution_plan<'a>(
        &self,
        to_install: &[String],
        cache: &'a FormulaCache,
    ) -> Result<ExecutionPlan<'a>, BrewdockError> {
        let entries = to_install
            .iter()
            .map(|name| {
                let formula = cache.get(name).ok_or_else(|| FormulaError::NotFound {
                    name: FormulaName::from(name.clone()),
                })?;
                let method =
                    self.instrument_phase("install", "resolve-install-method", name, || {
                        self.resolve_install_method(formula)
                    })?;
                Ok(ExecutionPlanEntry::new(formula, method))
            })
            .collect::<Result<Vec<_>, BrewdockError>>()?;

        Ok(ExecutionPlan {
            entries,
            acquire_concurrency: Self::MAX_ACQUIRE_CONCURRENCY,
        })
    }
}
impl<R: FormulaRepository, D: BottleDownloader> Orchestrator<R, D> {
    async fn instrument_operation<T, F>(
        &self,
        operation: &'static str,
        request: &[&str],
        future: F,
    ) -> Result<T, BrewdockError>
    where
        F: Future<Output = Result<T, BrewdockError>>,
    {
        let target = request_label(request);
        self.progress_sink.emit(ProgressEvent::OperationStarted {
            operation,
            target: target.clone(),
        });
        let result = future
            .instrument(tracing::info_span!(
                "bd.operation",
                operation,
                request = %target
            ))
            .await;
        match &result {
            Ok(_) => self
                .progress_sink
                .emit(ProgressEvent::OperationCompleted { operation, target }),
            Err(error) => self.progress_sink.emit(ProgressEvent::OperationFailed {
                operation,
                target,
                error: error.to_string(),
            }),
        }
        result
    }

    async fn instrument_async_phase<T, E, F>(
        &self,
        operation: &'static str,
        phase: &'static str,
        target: &str,
        future: F,
    ) -> Result<T, BrewdockError>
    where
        E: Into<BrewdockError>,
        F: Future<Output = Result<T, E>>,
    {
        let target_label = target.to_owned();
        self.progress_sink.emit(ProgressEvent::PhaseStarted {
            operation,
            phase,
            target: target_label.clone(),
        });
        let result = future
            .instrument(tracing::info_span!("bd.phase", operation, phase, target))
            .await
            .map_err(Into::into);
        match &result {
            Ok(_) => self.progress_sink.emit(ProgressEvent::PhaseCompleted {
                operation,
                phase,
                target: target_label,
            }),
            Err(error) => self.progress_sink.emit(ProgressEvent::PhaseFailed {
                operation,
                phase,
                target: target_label,
                error: error.to_string(),
            }),
        }
        result
    }

    fn instrument_phase<T, E, F>(
        &self,
        operation: &'static str,
        phase: &'static str,
        target: &str,
        work: F,
    ) -> Result<T, BrewdockError>
    where
        E: Into<BrewdockError>,
        F: FnOnce() -> Result<T, E>,
    {
        let target_label = target.to_owned();
        self.progress_sink.emit(ProgressEvent::PhaseStarted {
            operation,
            phase,
            target: target_label.clone(),
        });
        let span = tracing::info_span!("bd.phase", operation, phase, target);
        let _entered = span.enter();
        let result = work().map_err(Into::into);
        match &result {
            Ok(_) => self.progress_sink.emit(ProgressEvent::PhaseCompleted {
                operation,
                phase,
                target: target_label,
            }),
            Err(error) => self.progress_sink.emit(ProgressEvent::PhaseFailed {
                operation,
                phase,
                target: target_label,
                error: error.to_string(),
            }),
        }
        result
    }

    fn emit_warning(&self, operation: &'static str, target: &str, message: &str) {
        self.progress_sink.emit(ProgressEvent::Warning {
            operation,
            target: target.to_owned(),
            message: message.to_owned(),
        });
    }

    fn emit_formula_started(&self, operation: &'static str, name: &str) {
        self.progress_sink.emit(ProgressEvent::FormulaStarted {
            operation,
            name: name.to_owned(),
        });
    }

    fn emit_formula_completed(&self, operation: &'static str, name: &str) {
        self.progress_sink.emit(ProgressEvent::FormulaCompleted {
            operation,
            name: name.to_owned(),
        });
    }

    fn emit_formula_failed(&self, operation: &'static str, name: &str, error: &str) {
        self.progress_sink.emit(ProgressEvent::FormulaFailed {
            operation,
            name: name.to_owned(),
            error: error.to_owned(),
        });
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

fn request_label(names: &[&str]) -> String {
    if names.is_empty() {
        "all".to_owned()
    } else {
        names.join(",")
    }
}

/// Creates the package version string used in Cellar paths.
pub(crate) fn pkg_version(version: &str, revision: u32) -> String {
    if revision > 0 {
        format!("{version}_{revision}")
    } else {
        version.to_owned()
    }
}

/// Returns `true` when `installed` is strictly older than `latest`.
///
/// Falls back to string inequality if either version string cannot be
/// parsed.  A warning is emitted so unparseable versions are visible
/// in logs.
fn is_outdated(installed: &str, latest: &str) -> bool {
    if let (Ok(inst), Ok(lat)) = (
        installed.parse::<PkgVersion>(),
        latest.parse::<PkgVersion>(),
    ) {
        inst < lat
    } else {
        tracing::warn!(
            installed,
            latest,
            "falling back to string comparison: version parse failed"
        );
        installed != latest
    }
}

/// Walks a directory tree and removes entries whose names are not in
/// `keep_shas`.
///
/// For the blob store, top-level entries are 2-char prefix directories;
/// for the extracted store, they are SHA-based directories. In both
/// cases we check leaf names against `keep_shas`.
///
/// Returns `(entries_removed, bytes_freed)`.
fn cleanup_directory_tree(
    root: &Path,
    keep_shas: &HashSet<String>,
    dry_run: bool,
) -> Result<(u64, u64), BrewdockError> {
    let mut removed: u64 = 0;
    let mut freed: u64 = 0;

    for top_entry in std::fs::read_dir(root)?.flatten() {
        let top_path = top_entry.path();
        if top_path.is_dir() {
            for sub_entry in std::fs::read_dir(&top_path)?.flatten() {
                let sub_path = sub_entry.path();
                let file_name = sub_entry.file_name();
                let name = file_name.to_string_lossy();
                if !keep_shas.contains(name.as_ref()) {
                    freed += dir_size(&sub_path);
                    if !dry_run {
                        remove_path(&sub_path);
                    }
                    removed += 1;
                }
            }
            if !dry_run {
                let _ = std::fs::remove_dir(&top_path);
            }
        } else {
            let file_name = top_entry.file_name();
            let name = file_name.to_string_lossy();
            if !keep_shas.contains(name.as_ref()) {
                freed += top_entry.metadata().map_or(0, |m| m.len());
                if !dry_run {
                    remove_path(&top_path);
                }
                removed += 1;
            }
        }
    }

    Ok((removed, freed))
}

/// Removes a path regardless of whether it is a file or directory.
fn remove_path(path: &Path) {
    // Try remove_dir_all first; fall back to remove_file on error.
    if std::fs::remove_dir_all(path).is_err() {
        let _ = std::fs::remove_file(path);
    }
}

/// Returns the total size of a directory tree using a single metadata call per entry.
fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    let mut total: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let Ok(entry_meta) = entry.metadata() else {
                continue;
            };
            if entry_meta.is_dir() {
                total += dir_size(&entry.path());
            } else {
                total += entry_meta.len();
            }
        }
    }
    total
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

/// Detects the current platform context for Tier 2 DSL evaluation.
fn detect_platform() -> PlatformContext {
    detect_platform_context()
}

#[cfg(test)]
mod tests;
