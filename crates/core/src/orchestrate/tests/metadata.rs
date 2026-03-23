use super::*;

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
async fn test_update_writes_metadata_and_formulae() -> Result<(), Box<dyn std::error::Error>> {
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
    assert!(layout.cache_dir().join("formula.db").exists());

    let store = brewdock_formula::MetadataStore::new(layout.cache_dir());
    let meta = store.load_metadata()?.ok_or("metadata should exist")?;
    assert!(meta.fetched_at > 0);
    assert_eq!(meta.formula_count, 1);
    Ok(())
}

#[tokio::test]
async fn test_install_plan_uses_cached_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    // Pre-populate the disk cache using MetadataStore
    let store = brewdock_formula::MetadataStore::new(layout.cache_dir());
    store.save_index(
        &[
            make_formula("jq", "1.8.1", &["oniguruma"], PLAN_SHA),
            make_formula("oniguruma", "6.9.9", &[], PLAN_SHA),
        ],
        &brewdock_formula::IndexMetadata {
            etag: None,
            fetched_at: 0,
            formula_count: 2,
        },
    )?;

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
async fn test_install_plan_cache_miss_falls_back_to_network()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    // Pre-populate cache with only jq (missing oniguruma)
    let store = brewdock_formula::MetadataStore::new(layout.cache_dir());
    store.save_index(
        &[make_formula("jq", "1.8.1", &["oniguruma"], PLAN_SHA)],
        &brewdock_formula::IndexMetadata {
            etag: None,
            fetched_at: 0,
            formula_count: 1,
        },
    )?;

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
