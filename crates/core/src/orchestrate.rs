use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
};

use brewdock_bottle::{BlobStore, BottleDownloader, extract_tar_gz};
use brewdock_cellar::{
    InstallReason, InstallReceipt, InstallRecord, ReceiptDependency, ReceiptSource,
    ReceiptSourceVersions, StateDb, link, materialize, relocate_keg, unlink, write_receipt,
};
use brewdock_formula::{
    Formula, FormulaCache, FormulaError, FormulaRepository, SelectedBottle, UnsupportedReason,
    check_supportability, resolve_install_order, select_bottle,
};

use crate::{BrewdockError, HostTag, Layout, lock::FileLock};

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

            // Install new version. Only the formula itself is fetched; new
            // transitive dependencies introduced by the new version are not
            // resolved or installed here (see Non-goals: partial failure recovery).
            let cache = {
                let mut c = FormulaCache::new();
                c.insert(candidate.formula);
                c
            };
            // Preserve the original installed_on_request status from the record.
            let requested: HashSet<&str> = if candidate.record.installed_on_request {
                std::iter::once(candidate.record.name.as_str()).collect()
            } else {
                HashSet::new()
            };
            self.install_single(&candidate.record.name, &cache, &requested, &blob_store)
                .await?;

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

        let order = resolve_install_order(cache.all(), names)?;

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
            for dep in &formula.dependencies {
                if cache.get(dep).is_none() {
                    queue.push_back(dep.clone());
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

        if formula.post_install_defined {
            return Err(FormulaError::Unsupported {
                name: name.to_owned(),
                reason: UnsupportedReason::PostInstallDefined,
            }
            .into());
        }

        tracing::info!(name, "installing formula");

        let selected_bottle = match method {
            InstallMethod::Bottle(selected) => selected,
            InstallMethod::Source(_) => {
                return Err(FormulaError::Unsupported {
                    name: name.to_owned(),
                    reason: UnsupportedReason::SourceBuildRequired,
                }
                .into());
            }
        };

        // Download (async).
        let data = self
            .downloader
            .download_verified(&selected_bottle.url, &selected_bottle.sha256)
            .await?;

        // All operations below are sync. StateDb is opened here, after the
        // last .await, so the future remains Send.
        blob_store.put(&selected_bottle.sha256, &data)?;

        let extract_dir = self.layout.store_dir().join(&selected_bottle.sha256);
        let blob_path = blob_store.blob_path(&selected_bottle.sha256)?;
        extract_tar_gz(&blob_path, &extract_dir)?;

        let version_str = pkg_version(&formula.versions.stable, formula.revision);
        let source = extract_dir.join(&formula.name).join(&version_str);
        let keg_path = self.layout.cellar().join(&formula.name).join(&version_str);
        materialize(&source, &keg_path, &self.layout.opt_dir(), &formula.name)?;
        relocate_keg(&keg_path, self.layout.prefix())?;

        if !formula.keg_only {
            link(&keg_path, self.layout.prefix())?;
        }

        let is_requested = requested.contains(name);
        let receipt_deps = build_receipt_deps(formula, cache);
        let receipt_source = build_receipt_source(formula);
        let receipt = InstallReceipt::for_bottle(
            if is_requested {
                InstallReason::OnRequest
            } else {
                InstallReason::AsDependency
            },
            Some(unix_timestamp_f64()),
            receipt_deps,
            receipt_source,
        );
        write_receipt(&keg_path, &receipt)?;

        let state_db = StateDb::open(&self.layout.db_path())?;
        state_db.insert(&InstallRecord {
            name: formula.name.clone(),
            version: formula.versions.stable.clone(),
            revision: formula.revision,
            installed_on_request: is_requested,
            installed_at: unix_timestamp_string(),
        })?;

        tracing::info!(name, "installation complete");
        Ok(())
    }

    fn resolve_install_method(&self, formula: &Formula) -> Result<InstallMethod, FormulaError> {
        if let Some(selected) = select_bottle(formula, self.host_tag.as_str()) {
            return Ok(InstallMethod::Bottle(selected));
        }

        build_source_plan(formula, &self.layout)
            .map(InstallMethod::Source)
            .ok_or_else(|| FormulaError::Unsupported {
                name: formula.name.clone(),
                reason: UnsupportedReason::NoBottleForTag(self.host_tag.to_string()),
            })
    }
}

fn build_source_plan(formula: &Formula, layout: &Layout) -> Option<SourceBuildPlan> {
    let stable = formula.urls.stable.as_ref()?;
    let version = pkg_version(&formula.versions.stable, formula.revision);
    Some(SourceBuildPlan {
        formula_name: formula.name.clone(),
        version: version.clone(),
        source_url: stable.url.clone(),
        source_checksum: stable.checksum.clone(),
        build_dependencies: formula.build_dependencies.clone(),
        runtime_dependencies: formula.dependencies.clone(),
        prefix: layout.prefix().to_path_buf(),
        cellar_path: layout.cellar().join(&formula.name).join(version),
    })
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
    }

    impl MockRepo {
        fn new(list: Vec<Formula>) -> Self {
            let formulae = list.into_iter().map(|f| (f.name.clone(), f)).collect();
            Self { formulae }
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

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout.clone())?;

        let plan = orchestrator.plan_install(&["a"]).await?;
        let entry = plan.first().ok_or("expected install plan entry")?;
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
}
