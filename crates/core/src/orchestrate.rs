use std::collections::{HashSet, VecDeque};

use brewdock_bottle::{BlobStore, BottleDownloader, extract_tar_gz};
use brewdock_cellar::{
    InstallReceipt, InstallRecord, ReceiptDependency, ReceiptSource, ReceiptSourceVersions,
    StateDb, link, materialize, unlink, write_receipt,
};
use brewdock_formula::{
    Formula, FormulaCache, FormulaError, FormulaRepository, error::UnsupportedReason,
    resolve::resolve_install_order, supportability::check_supportability,
};

use crate::{BrewdockError, HostTag, Layout, lock::FileLock};

/// Default tap name for receipt source metadata.
const TAP_NAME: &str = "homebrew/core";

/// Prefix for formula source paths in receipt metadata.
const FORMULA_PATH_PREFIX: &str = "@@HOMEBREW_PREFIX@@/Library/Taps/homebrew/homebrew-core/Formula";

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

    /// Installs the requested formulae and all their dependencies.
    ///
    /// Returns the names of formulae actually installed (excludes already-installed).
    ///
    /// # Errors
    ///
    /// Returns [`BrewdockError`] if any step fails. Unsupported formulae are
    /// rejected before any download begins.
    pub async fn install(&self, names: &[String]) -> Result<Vec<String>, BrewdockError> {
        let _lock = self.acquire_lock()?;

        // Fetch all formulae + transitive deps.
        let cache = self.fetch_with_deps(names).await?;

        // Check supportability for ALL before any download.
        let host_tag = self.host_tag.as_str();
        for formula in cache.all().values() {
            check_supportability(formula, host_tag)?;
        }

        // Resolve topological install order.
        let order = resolve_install_order(cache.all(), names)?;

        // Filter already-installed. StateDb is dropped before the async loop.
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

        // Install each in order.
        let requested: HashSet<&str> = names.iter().map(String::as_str).collect();
        let blob_store = BlobStore::new(&self.layout.blob_dir());
        for name in &to_install {
            self.install_single(name, &cache, &requested, &blob_store)
                .await?;
        }

        Ok(to_install)
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
    pub async fn upgrade(&self, names: &[String]) -> Result<Vec<String>, BrewdockError> {
        let _lock = self.acquire_lock()?;

        // Get installed records. StateDb is dropped before the async loop.
        let installed = {
            let state_db = StateDb::open(&self.layout.db_path())?;
            if names.is_empty() {
                state_db.list()?
            } else {
                let mut records = Vec::new();
                for name in names {
                    if let Some(record) = state_db.get(name)? {
                        records.push(record);
                    }
                }
                records
            }
        };

        let mut upgraded = Vec::new();
        let blob_store = BlobStore::new(&self.layout.blob_dir());

        for record in &installed {
            let formula = self.repo.get_formula(&record.name).await?;
            let host_tag = self.host_tag.as_str();
            check_supportability(&formula, host_tag)?;

            let current_version = pkg_version(&record.version, record.revision);
            let latest_version = pkg_version(&formula.versions.stable, formula.revision);
            if current_version == latest_version {
                continue;
            }

            tracing::info!(
                name = record.name,
                from = current_version,
                to = latest_version,
                "upgrading formula"
            );

            // Unlink old keg. The old keg directory is intentionally kept in the
            // Cellar for potential rollback; cleanup is a separate concern.
            let old_keg = self
                .layout
                .cellar()
                .join(&record.name)
                .join(&current_version);
            if old_keg.exists() {
                unlink(&old_keg, self.layout.prefix())?;
            }

            // Install new version. Only the formula itself is fetched; new
            // transitive dependencies introduced by the new version are not
            // resolved or installed here (see Non-goals: partial failure recovery).
            let cache = {
                let mut c = FormulaCache::new();
                c.insert(formula);
                c
            };
            // Preserve the original installed_on_request status from the record.
            let requested: HashSet<&str> = if record.installed_on_request {
                std::iter::once(record.name.as_str()).collect()
            } else {
                HashSet::new()
            };
            self.install_single(&record.name, &cache, &requested, &blob_store)
                .await?;

            upgraded.push(record.name.clone());
        }

        Ok(upgraded)
    }

    /// Acquires the brewdock file lock.
    fn acquire_lock(&self) -> Result<FileLock, std::io::Error> {
        FileLock::acquire(&self.layout.lock_dir().join("brewdock.lock"))
    }

    /// Fetches formulae and all transitive dependencies.
    async fn fetch_with_deps(&self, names: &[String]) -> Result<FormulaCache, BrewdockError> {
        let mut cache = FormulaCache::new();
        let mut queue: VecDeque<String> = names.iter().cloned().collect();

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

        let host_tag = self.host_tag.as_str();
        let bottle_stable =
            formula
                .bottle
                .stable
                .as_ref()
                .ok_or_else(|| FormulaError::Unsupported {
                    name: name.to_owned(),
                    reason: UnsupportedReason::NoBottle,
                })?;
        let bottle_file =
            bottle_stable
                .files
                .get(host_tag)
                .ok_or_else(|| FormulaError::Unsupported {
                    name: name.to_owned(),
                    reason: UnsupportedReason::NoBottleForTag(host_tag.to_owned()),
                })?;

        tracing::info!(name, "installing formula");

        // Download (async).
        let data = self
            .downloader
            .download_verified(&bottle_file.url, &bottle_file.sha256)
            .await?;

        // All operations below are sync. StateDb is opened here, after the
        // last .await, so the future remains Send.
        blob_store.put(&bottle_file.sha256, &data)?;

        let extract_dir = self.layout.store_dir().join(&bottle_file.sha256);
        extract_tar_gz(&blob_store.blob_path(&bottle_file.sha256), &extract_dir)?;

        let version_str = pkg_version(&formula.versions.stable, formula.revision);
        let source = extract_dir.join(&formula.name).join(&version_str);
        let keg_path = self.layout.cellar().join(&formula.name).join(&version_str);
        materialize(&source, &keg_path, &self.layout.opt_dir(), &formula.name)?;

        if !formula.keg_only {
            link(&keg_path, self.layout.prefix())?;
        }

        let is_requested = requested.contains(name);
        let receipt_deps = build_receipt_deps(formula, cache);
        let receipt_source = build_receipt_source(formula);
        let receipt = InstallReceipt::for_bottle(
            !is_requested,
            is_requested,
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
        CellarType,
        types::{BottleFile, BottleSpec, BottleStable, Versions},
    };

    use super::*;

    const HOST_TAG: &str = "arm64_sequoia";

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
            pour_bottle_only_if: None,
            keg_only: false,
            dependencies: deps.iter().map(|s| (*s).to_owned()).collect(),
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
    ) -> Orchestrator<MockRepo, MockDownloader> {
        // SAFETY: HOST_TAG is a valid host tag constant.
        #[expect(clippy::unwrap_used, reason = "test-only constant parsing")]
        let host_tag: HostTag = HOST_TAG.parse().ok().unwrap();
        let repo = MockRepo::new(formulae);
        let downloader = MockDownloader::new(bottles, counter);
        Orchestrator::new(repo, downloader, layout, host_tag)
    }

    // --- Install tests ---

    #[tokio::test]
    async fn test_install_resolves_and_installs_in_topological_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha_a = "sha256_a";
        let sha_b = "sha256_b";
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
        );

        let installed = orchestrator.install(&["a".to_owned()]).await?;

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

        let sha_a = "sha256_a";
        let sha_b = "sha256_b";
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
        );

        let installed = orchestrator.install(&["a".to_owned()]).await?;

        // Only "a" should be installed; "b" is skipped.
        assert_eq!(installed, vec!["a"]);

        // Only one download (for "a").
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn test_install_rejects_unsupported_before_download()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let mut formula = make_formula("disabled_pkg", "1.0", &[], "sha_disabled");
        formula.disabled = true;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula], vec![], counter.clone(), layout.clone());

        let result = orchestrator.install(&["disabled_pkg".to_owned()]).await;

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
        let sha = "sha256_new";
        let formula = make_formula("a", "2.0", &[], sha);
        let tar = create_bottle_tar_gz("a", "2.0", &[("bin/a_tool", b"new_version")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula],
            vec![(sha, tar)],
            counter.clone(),
            layout.clone(),
        );

        let upgraded = orchestrator.upgrade(&["a".to_owned()]).await?;

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

        let formula = make_formula("a", "1.0", &[], "sha_same");
        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula], vec![], counter.clone(), layout.clone());

        let upgraded = orchestrator.upgrade(&["a".to_owned()]).await?;

        // Nothing upgraded.
        assert!(upgraded.is_empty());
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        Ok(())
    }
}
