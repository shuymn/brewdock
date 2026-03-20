use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};

use brewdock_bottle::{BlobStore, BottleDownloader, extract_tar_gz};
use brewdock_cellar::{
    InstallReason, InstallReceipt, InstallRecord, PostInstallContext, PostInstallTransaction,
    ReceiptDependency, ReceiptSource, ReceiptSourceVersions, StateDb, atomic_symlink_replace, link,
    materialize, relocate_keg, run_post_install, unlink, write_receipt,
};
use brewdock_formula::{
    CellarType, Formula, FormulaCache, FormulaError, FormulaRepository, Requirement,
    SelectedBottle, UnsupportedReason, check_supportability, resolve_install_order, select_bottle,
};

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

/// Orchestrates formula installation and upgrade operations.
///
/// Generic over `R` (formula repository) and `D` (bottle downloader) for
/// testability via mock implementations.
///
/// Blocking I/O (file operations, `SQLite`) is called directly in async methods.
/// This is acceptable because the intended runtime is single-threaded
/// (`current_thread`) with no concurrent async work.
pub struct Orchestrator<R, D> {
    repo: R,
    downloader: D,
    layout: Layout,
    host_tag: HostTag,
}

#[derive(Debug, Clone)]
struct UpgradeCandidate {
    record: InstallRecord,
    formula: Formula,
    current_version: String,
    latest_version: String,
    method: InstallMethod,
}

impl<R: FormulaRepository, D: BottleDownloader> Orchestrator<R, D> {
    /// Creates a new orchestrator.
    #[must_use]
    pub const fn new(repo: R, downloader: D, layout: Layout, host_tag: HostTag) -> Self {
        Self {
            repo,
            downloader,
            layout,
            host_tag,
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
        let _lock = self.acquire_lock()?;
        let (to_install, cache) = self.resolve_install_list(names).await?;
        let entries = to_install
            .iter()
            .map(|name| {
                let f = cache
                    .get(name)
                    .ok_or_else(|| FormulaError::NotFound { name: name.clone() })?;
                Ok(PlanEntry {
                    name: name.clone(),
                    version: pkg_version(&f.versions.stable, f.revision),
                    method: self.resolve_install_method(f)?,
                })
            })
            .collect::<Result<Vec<_>, BrewdockError>>()?;
        Ok(entries)
    }

    /// Installs the requested formulae and all their dependencies.
    ///
    /// Returns the names of formulae actually installed (excludes already-installed).
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if any step fails. Unsupported formulae are
    /// rejected before any download begins.
    pub async fn install(&self, names: &[&str]) -> Result<Vec<String>, BrewdockError> {
        let _lock = self.acquire_lock()?;
        let (to_install, cache) = self.resolve_install_list(names).await?;

        // Install each in order.
        let requested: HashSet<&str> = names.iter().copied().collect();
        let blob_store = BlobStore::new(&self.layout.blob_dir());
        for name in &to_install {
            self.install_single(name, &cache, &requested, &blob_store)
                .await?;
        }

        Ok(to_install)
    }

    /// Fetches the formula index and caches it locally.
    ///
    /// Returns the number of formulae cached.
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if the fetch or file write fails.
    pub async fn update(&self) -> Result<usize, BrewdockError> {
        let formulae = self.repo.get_all_formulae().await?;
        let count = formulae.len();
        let json = serde_json::to_vec(&formulae).map_err(FormulaError::from)?;
        let cache_dir = self.layout.cache_dir();
        std::fs::create_dir_all(&cache_dir)?;
        std::fs::write(cache_dir.join("formula.json"), json)?;
        tracing::info!(count, "formula index cached");
        Ok(count)
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
        let _lock = self.acquire_lock()?;
        let candidates = self.collect_upgrade_candidates(names).await?;
        Ok(candidates
            .into_iter()
            .map(|candidate| UpgradePlanEntry {
                name: candidate.record.name,
                from_version: candidate.current_version,
                to_version: candidate.latest_version,
                method: candidate.method,
            })
            .collect())
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
        let _lock = self.acquire_lock()?;
        let candidates = self.collect_upgrade_candidates(names).await?;

        let mut upgraded = Vec::new();
        let blob_store = BlobStore::new(&self.layout.blob_dir());

        for candidate in candidates {
            tracing::info!(
                name = candidate.record.name,
                from = candidate.current_version,
                to = candidate.latest_version,
                "upgrading formula"
            );

            // Unlink old keg. The old keg directory is intentionally kept in the
            // Cellar for potential rollback; cleanup is a separate concern.
            let old_keg = self
                .layout
                .cellar()
                .join(&candidate.record.name)
                .join(&candidate.current_version);
            if old_keg.exists() {
                unlink(&old_keg, self.layout.prefix())?;
            }

            let requested: HashSet<&str> = if candidate.record.installed_on_request {
                std::iter::once(candidate.record.name.as_str()).collect()
            } else {
                HashSet::new()
            };

            if matches!(candidate.method, InstallMethod::Source(_)) {
                let (to_install, cache) = self.resolve_upgrade_install_list(&candidate).await?;
                for name in &to_install {
                    self.install_single(name, &cache, &requested, &blob_store)
                        .await?;
                }
            } else {
                let cache = {
                    let mut c = FormulaCache::new();
                    c.insert(candidate.formula);
                    c
                };
                self.install_single(&candidate.record.name, &cache, &requested, &blob_store)
                    .await?;
            }

            upgraded.push(candidate.record.name);
        }

        Ok(upgraded)
    }

    /// Fetches installed records from the state database.
    ///
    /// If `names` is empty, returns all installed records. Otherwise, returns
    /// only records matching the given names. `StateDb` is opened and dropped
    /// within this call to avoid holding it across `.await` boundaries.
    fn fetch_installed_records(&self, names: &[&str]) -> Result<Vec<InstallRecord>, BrewdockError> {
        let state_db = StateDb::open(&self.layout.db_path())?;
        if names.is_empty() {
            Ok(state_db.list()?)
        } else {
            let mut records = Vec::new();
            for &name in names {
                if let Some(record) = state_db.get(name)? {
                    records.push(record);
                }
            }
            Ok(records)
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
            let state_db = StateDb::open(&self.layout.db_path())?;
            let mut pending = Vec::new();
            for name in order {
                if state_db.get(&name)?.is_none() {
                    pending.push(name);
                }
            }
            pending
        };

        Ok((to_install, cache))
    }

    /// Acquires the brewdock file lock.
    fn acquire_lock(&self) -> Result<FileLock, std::io::Error> {
        FileLock::acquire(&self.layout.lock_dir().join("brewdock.lock"))
    }

    /// Fetches formulae and all transitive dependencies.
    async fn fetch_with_deps(&self, names: &[&str]) -> Result<FormulaCache, BrewdockError> {
        let mut cache = FormulaCache::new();
        let mut queue: VecDeque<String> = names.iter().map(|name| (*name).to_owned()).collect();

        while let Some(name) = queue.pop_front() {
            if cache.get(&name).is_some() {
                continue;
            }
            let formula = self.repo.get_formula(&name).await?;
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
        let installed = self.fetch_installed_records(names)?;
        let host_tag = self.host_tag.as_str();
        let mut candidates = Vec::new();

        for record in installed {
            let formula = self.repo.get_formula(&record.name).await?;
            check_supportability(&formula, host_tag)?;
            let method = self.resolve_install_method(&formula)?;

            let current_version = pkg_version(&record.version, record.revision);
            let latest_version = pkg_version(&formula.versions.stable, formula.revision);
            if current_version == latest_version {
                continue;
            }

            candidates.push(UpgradeCandidate {
                record,
                formula,
                current_version,
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
        let cache = self
            .fetch_with_deps(&[candidate.record.name.as_str()])
            .await?;
        let host_tag = self.host_tag.as_str();
        for formula in cache.all().values() {
            check_supportability(formula, host_tag)?;
        }

        let order = resolve_install_order(
            &self.build_install_graph(&cache)?,
            &[candidate.record.name.as_str()],
        )?;
        let state_db = StateDb::open(&self.layout.db_path())?;
        let mut to_install = Vec::new();
        for name in order {
            if name == candidate.record.name || state_db.get(&name)?.is_none() {
                to_install.push(name);
            }
        }
        Ok((to_install, cache))
    }

    /// Installs a single formula (download → extract → materialize → link → receipt → state).
    ///
    /// `StateDb` is opened after the download completes to avoid holding a non-`Send`
    /// reference across `.await` boundaries.
    async fn install_single(
        &self,
        name: &str,
        cache: &FormulaCache,
        requested: &HashSet<&str>,
        blob_store: &BlobStore,
    ) -> Result<(), BrewdockError> {
        let formula = cache.get(name).ok_or_else(|| FormulaError::NotFound {
            name: name.to_owned(),
        })?;
        let method = self.resolve_install_method(formula)?;

        tracing::info!(name, "installing formula");

        let keg_path = self
            .install_payload(formula, &method, blob_store)
            .await
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
        let post_install_transaction = self
            .execute_post_install(name, formula, &keg_path)
            .await
            .inspect_err(|_| {
                let _ = cleanup_failed_install(
                    &keg_path,
                    self.layout.prefix(),
                    &self.layout.opt_dir(),
                    &formula.name,
                );
            })?;

        let is_requested = requested.contains(formula.name.as_str());
        let receipt = build_receipt(
            &method,
            if is_requested {
                InstallReason::OnRequest
            } else {
                InstallReason::AsDependency
            },
            Some(unix_timestamp_f64()),
            build_receipt_deps(formula, cache),
            build_receipt_source(formula),
        );
        let finalize_install =
            self.finalize_installed_formula(formula, &keg_path, &receipt, is_requested);

        if let Err(error) = finalize_install {
            if let Some(transaction) = post_install_transaction {
                transaction.rollback()?;
            }
            cleanup_failed_install(
                &keg_path,
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

    async fn install_payload(
        &self,
        formula: &Formula,
        method: &InstallMethod,
        blob_store: &BlobStore,
    ) -> Result<PathBuf, BrewdockError> {
        match method {
            InstallMethod::Bottle(selected_bottle) => {
                let data = self
                    .downloader
                    .download_verified(&selected_bottle.url, &selected_bottle.sha256)
                    .await?;

                blob_store.put(&selected_bottle.sha256, &data)?;

                let extract_dir = self.layout.store_dir().join(&selected_bottle.sha256);
                let blob_path = blob_store.blob_path(&selected_bottle.sha256)?;
                extract_tar_gz(&blob_path, &extract_dir)?;

                let version_str = pkg_version(&formula.versions.stable, formula.revision);
                let source = extract_dir.join(&formula.name).join(&version_str);
                let keg_path = self.layout.cellar().join(&formula.name).join(&version_str);
                materialize(&source, &keg_path, &self.layout.opt_dir(), &formula.name)?;
                if !matches!(selected_bottle.cellar, CellarType::AnySkipRelocation) {
                    relocate_keg(&keg_path, self.layout.prefix())?;
                }
                Ok(keg_path)
            }
            InstallMethod::Source(plan) => self.install_source_formula(formula, plan).await,
        }
    }

    async fn execute_post_install(
        &self,
        name: &str,
        formula: &Formula,
        keg_path: &Path,
    ) -> Result<Option<PostInstallTransaction>, BrewdockError> {
        if !formula.post_install_defined {
            return Ok(None);
        }

        let ruby_source_path =
            formula
                .ruby_source_path
                .as_deref()
                .ok_or_else(|| FormulaError::Unsupported {
                    name: name.to_owned(),
                    reason: UnsupportedReason::PostInstallDefined,
                })?;
        let ruby_source = self.repo.get_ruby_source(ruby_source_path).await?;
        let transaction = run_post_install(
            &ruby_source,
            &PostInstallContext::new(self.layout.prefix(), keg_path),
        )?;
        Ok(Some(transaction))
    }

    fn finalize_installed_formula(
        &self,
        formula: &Formula,
        keg_path: &Path,
        receipt: &InstallReceipt,
        is_requested: bool,
    ) -> Result<(), BrewdockError> {
        if !formula.keg_only {
            link(keg_path, self.layout.prefix())?;
        }

        write_receipt(keg_path, receipt)?;

        let state_db = StateDb::open(&self.layout.db_path())?;
        state_db.insert(&InstallRecord {
            name: formula.name.clone(),
            version: formula.versions.stable.clone(),
            revision: formula.revision,
            installed_on_request: is_requested,
            installed_at: unix_timestamp_string(),
        })?;
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
                    name: formula.name.clone(),
                    reason: UnsupportedReason::IncompatibleCellar(bottle.cellar.to_string()),
                }
                .into());
            }
        } else if formula.urls.stable.is_none() {
            return Err(FormulaError::Unsupported {
                name: formula.name.clone(),
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
    let source_checksum = stable
        .checksum
        .clone()
        .ok_or_else(|| SourceBuildError::MissingSourceChecksum(formula.name.clone()))?;
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
    async fn install_source_formula(
        &self,
        _formula: &Formula,
        plan: &SourceBuildPlan,
    ) -> Result<PathBuf, BrewdockError> {
        let checksum = plan
            .source_checksum
            .as_deref()
            .ok_or_else(|| SourceBuildError::MissingSourceChecksum(plan.formula_name.clone()))?;
        let data = self
            .downloader
            .download_verified(&plan.source_url, checksum)
            .await?;

        let parent = self.layout.cache_dir().join("sources");
        std::fs::create_dir_all(&parent)?;
        let tempdir = tempfile::tempdir_in(parent)?;
        let archive_path = tempdir
            .path()
            .join(source_archive_filename(&plan.source_url).unwrap_or("source.tar.gz"));
        std::fs::write(&archive_path, data)?;

        let source_root = extract_source_archive(&archive_path, tempdir.path())?;
        run_source_build(&source_root, plan, self.layout.prefix())?;
        refresh_opt_link(
            &plan.cellar_path,
            &self.layout.opt_dir(),
            &plan.formula_name,
        )?;
        Ok(plan.cellar_path.clone())
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

/// Returns the current Unix timestamp as a string of seconds.
fn unix_timestamp_string() -> String {
    unix_now().as_secs().to_string()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use brewdock_bottle::BottleError;
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
        async fn get_formula(&self, name: &str) -> Result<Formula, FormulaError> {
            self.formulae
                .get(name)
                .cloned()
                .ok_or_else(|| FormulaError::NotFound {
                    name: name.to_owned(),
                })
        }

        async fn get_all_formulae(&self) -> Result<Vec<Formula>, FormulaError> {
            Ok(self.formulae.values().cloned().collect())
        }

        async fn get_ruby_source(&self, ruby_source_path: &str) -> Result<String, FormulaError> {
            self.ruby_sources
                .get(ruby_source_path)
                .cloned()
                .ok_or_else(|| FormulaError::NotFound {
                    name: ruby_source_path.to_owned(),
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

        // Verify state DB.
        let state_db = StateDb::open(&layout.db_path())?;
        let rec_a = state_db.get("a")?.ok_or("expected record for a")?;
        assert_eq!(rec_a.version, "1.0");
        assert!(rec_a.installed_on_request);
        let rec_b = state_db.get("b")?.ok_or("expected record for b")?;
        assert_eq!(rec_b.version, "2.0");
        assert!(!rec_b.installed_on_request);

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

        // Pre-populate state DB with "b" already installed.
        let state_db = StateDb::open(&layout.db_path())?;
        state_db.insert(&InstallRecord {
            name: "b".to_owned(),
            version: "2.0".to_owned(),
            revision: 0,
            installed_on_request: false,
            installed_at: "1000".to_owned(),
        })?;
        drop(state_db);

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

        let state_db = StateDb::open(&layout.db_path())?;
        state_db.insert(&InstallRecord {
            name: "a".to_owned(),
            version: "1.0".to_owned(),
            revision: 0,
            installed_on_request: true,
            installed_at: "1000".to_owned(),
        })?;
        drop(state_db);

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
        assert!(StateDb::open(&layout.db_path())?.get("demo")?.is_some());
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
        assert!(StateDb::open(&layout.db_path())?.get("demo")?.is_none());
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
        assert!(
            StateDb::open(&layout.db_path())?
                .get("ca-certificates")?
                .is_some()
        );
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
        assert!(StateDb::open(&layout.db_path())?.get("target")?.is_some());
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
        assert!(StateDb::open(&layout.db_path())?.get("broken")?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_source_fallback_plan_upgrade_reuses_source_method_resolution()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let state_db = StateDb::open(&layout.db_path())?;
        state_db.insert(&InstallRecord {
            name: "portable".to_owned(),
            version: "1.0".to_owned(),
            revision: 0,
            installed_on_request: true,
            installed_at: "1000".to_owned(),
        })?;
        drop(state_db);

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

        let state_db = StateDb::open(&layout.db_path())?;
        state_db.insert(&InstallRecord {
            name: "target".to_owned(),
            version: "1.0".to_owned(),
            revision: 0,
            installed_on_request: true,
            installed_at: "1000".to_owned(),
        })?;
        drop(state_db);

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
        assert!(
            StateDb::open(&layout.db_path())?
                .get("build-helper")?
                .is_some()
        );
        assert_eq!(
            StateDb::open(&layout.db_path())?
                .get("target")?
                .ok_or("expected upgraded target record")?
                .version,
            "2.0"
        );
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

        // Pre-populate state DB.
        let state_db = StateDb::open(&layout.db_path())?;
        state_db.insert(&InstallRecord {
            name: "a".to_owned(),
            version: "1.0".to_owned(),
            revision: 0,
            installed_on_request: true,
            installed_at: "1000".to_owned(),
        })?;
        drop(state_db);

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

        // State DB updated, preserving installed_on_request status.
        let state_db = StateDb::open(&layout.db_path())?;
        let record = state_db.get("a")?.ok_or("expected record")?;
        assert_eq!(record.version, "2.0");
        assert!(record.installed_on_request);
        Ok(())
    }

    #[tokio::test]
    async fn test_upgrade_skips_current_version() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        // Pre-populate state DB with v1.0 (same as repo).
        let state_db = StateDb::open(&layout.db_path())?;
        state_db.insert(&InstallRecord {
            name: "a".to_owned(),
            version: "1.0".to_owned(),
            revision: 0,
            installed_on_request: true,
            installed_at: "1000".to_owned(),
        })?;
        drop(state_db);

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
    async fn test_install_any_skip_relocation_bottle_skips_relocation()
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
}
