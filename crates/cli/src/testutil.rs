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
    CellarType, Formula, FormulaError,
    types::{BottleFile, BottleSpec, BottleStable, Versions},
};

pub const HOST_TAG: &str = "arm64_sequoia";

pub struct MockRepo {
    pub formulae: HashMap<String, Formula>,
}

impl MockRepo {
    pub fn new(list: Vec<Formula>) -> Self {
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

pub fn make_orchestrator(
    formulae: Vec<Formula>,
    bottles: Vec<(&str, Vec<u8>)>,
    counter: Arc<AtomicUsize>,
    layout: Layout,
) -> Orchestrator<MockRepo, MockDownloader> {
    #[expect(clippy::unwrap_used, reason = "test-only constant parsing")]
    let host_tag: HostTag = HOST_TAG.parse().ok().unwrap();
    let repo = MockRepo::new(formulae);
    let downloader = MockDownloader::new(bottles, counter);
    Orchestrator::new(repo, downloader, layout, host_tag)
}
