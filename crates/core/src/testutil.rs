use std::{
    collections::HashMap,
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use brewdock_bottle::BottleError;
use brewdock_cellar::{
    InstallReason, InstallReceipt, ReceiptSource, ReceiptSourceVersions, atomic_symlink_replace,
    find_installed_keg, write_receipt,
};
use brewdock_formula::{
    BottleFile, BottleSpec, BottleStable, CellarType, FetchOutcome, Formula, FormulaError,
    FormulaName, FormulaRepository, FormulaUrls, StableUrl, Versions,
};

use crate::{BottleDownloader, BrewdockError, HostTag, Layout, Orchestrator};

pub const HOST_TAG: &str = "arm64_sequoia";
pub const PLAN_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

pub struct MockRepo {
    formulae: HashMap<String, Formula>,
    ruby_sources: HashMap<String, String>,
}

impl MockRepo {
    pub fn new(list: Vec<Formula>) -> Self {
        let formulae = list
            .into_iter()
            .map(|formula| (formula.name.clone(), formula))
            .collect();
        Self {
            formulae,
            ruby_sources: HashMap::new(),
        }
    }

    pub fn with_sources(list: Vec<Formula>, ruby_sources: HashMap<String, String>) -> Self {
        let formulae = list
            .into_iter()
            .map(|formula| (formula.name.clone(), formula))
            .collect();
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

    async fn all_formulae_conditional(
        &self,
        _etag: Option<&str>,
    ) -> Result<FetchOutcome, FormulaError> {
        let formulae = self.all_formulae().await?;
        Ok(FetchOutcome::Modified {
            formulae,
            etag: None,
        })
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

pub struct MockDownloader {
    data: HashMap<String, Vec<u8>>,
    download_count: Arc<AtomicUsize>,
}

impl MockDownloader {
    pub fn new(entries: Vec<(&str, Vec<u8>)>, counter: Arc<AtomicUsize>) -> Self {
        let data = entries
            .into_iter()
            .map(|(checksum, bytes)| (checksum.to_owned(), bytes))
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

pub fn make_formula(name: &str, version: &str, deps: &[&str], sha256: &str) -> Formula {
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
        dependencies: deps
            .iter()
            .map(|dependency| (*dependency).to_owned())
            .collect(),
        build_dependencies: Vec::new(),
        uses_from_macos: Vec::new(),
        requirements: Vec::new(),
        disabled: false,
        post_install_defined: false,
    }
}

/// Creates a tar.gz archive mimicking a Homebrew bottle structure.
pub fn create_bottle_tar_gz(
    name: &str,
    version: &str,
    files: &[(&str, &[u8])],
) -> Result<Vec<u8>, Box<dyn Error>> {
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

pub fn create_source_tar_gz(
    root: &str,
    files: &[(&str, &[u8])],
) -> Result<Vec<u8>, Box<dyn Error>> {
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

pub fn create_source_tar_gz_with_raw_paths(
    entries: &[(&[u8], &[u8])],
) -> Result<Vec<u8>, Box<dyn Error>> {
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

pub fn make_orchestrator(
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

pub fn make_orchestrator_with_sources(
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

pub fn move_host_bottle_to_tag(formula: &mut Formula, tag: &str) -> Result<(), Box<dyn Error>> {
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
pub fn setup_installed_keg(
    layout: &Layout,
    name: &str,
    version: &str,
    on_request: bool,
) -> Result<(), Box<dyn Error>> {
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

    std::fs::create_dir_all(layout.opt_dir())?;
    let opt_link = layout.opt_dir().join(name);
    atomic_symlink_replace(&keg_path, &opt_link)?;
    Ok(())
}

pub fn create_simple_bottle(name: &str, version: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    create_bottle_tar_gz(name, version, &[("bin/tool", b"#!/bin/sh\necho ok\n")])
}

/// Asserts that a formula is visible as installed via filesystem state.
pub fn assert_installed(layout: &Layout, name: &str) {
    assert!(
        find_installed_keg(name, &layout.cellar(), &layout.opt_dir())
            .ok()
            .flatten()
            .is_some(),
        "expected {name} to be installed"
    );
}

/// Asserts that a formula is NOT visible as installed via filesystem state.
pub fn assert_not_installed(layout: &Layout, name: &str) {
    assert!(
        find_installed_keg(name, &layout.cellar(), &layout.opt_dir())
            .ok()
            .flatten()
            .is_none(),
        "expected {name} to not be installed"
    );
}
