use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use brewdock_bottle::BottleError;
use brewdock_core::{BottleDownloader, FormulaRepository, HostTag, Layout, Orchestrator};
use brewdock_formula::{
    BottleFile, BottleSpec, BottleStable, CellarType, Formula, FormulaError, FormulaName,
    FormulaUrls, StableUrl, Versions,
};

pub const HOST_TAG: &str = "arm64_sequoia";
pub const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

pub struct MockRepo {
    pub formulae: HashMap<String, Formula>,
    pub ruby_sources: HashMap<String, String>,
}

impl MockRepo {
    pub fn new(list: Vec<Formula>) -> Self {
        let formulae = list.into_iter().map(|f| (f.name.clone(), f)).collect();
        Self {
            formulae,
            ruby_sources: HashMap::new(),
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

pub struct MockDownloader {
    data: HashMap<String, Vec<u8>>,
    download_count: Arc<AtomicUsize>,
}

impl MockDownloader {
    pub fn new(entries: Vec<(&str, Vec<u8>)>, counter: Arc<AtomicUsize>) -> Self {
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
        dependencies: deps.iter().map(|s| (*s).to_owned()).collect(),
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

/// Sets up filesystem install state for a formula (keg directory + receipt + opt symlink).
pub fn setup_installed_keg(
    layout: &Layout,
    name: &str,
    version: &str,
    on_request: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use brewdock_cellar::{
        InstallReason, InstallReceipt, ReceiptSource, ReceiptSourceVersions,
        atomic_symlink_replace, write_receipt,
    };

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

pub fn make_orchestrator(
    formulae: Vec<Formula>,
    bottles: Vec<(&str, Vec<u8>)>,
    counter: Arc<AtomicUsize>,
    layout: Layout,
) -> Result<Orchestrator<MockRepo, MockDownloader>, brewdock_core::error::BrewdockError> {
    let host_tag: HostTag = HOST_TAG.parse()?;
    let repo = MockRepo::new(formulae);
    let downloader = MockDownloader::new(bottles, counter);
    Ok(Orchestrator::new(repo, downloader, layout, host_tag))
}
