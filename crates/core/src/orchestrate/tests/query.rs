use super::*;

#[tokio::test]
async fn test_outdated_returns_outdated_formulae() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    setup_installed_keg(&layout, "a", "1.0", true)?;
    setup_installed_keg(&layout, "b", "2.0", true)?;

    let formulae = vec![
        make_formula("a", "2.0", &[], PLAN_SHA),
        make_formula("b", "2.0", &[], PLAN_SHA),
    ];
    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(formulae, vec![], counter, layout)?;

    let entries = orchestrator.outdated(&[]).await?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "a");
    assert_eq!(entries[0].current_version, "1.0");
    assert_eq!(entries[0].latest_version, "2.0");
    Ok(())
}

#[tokio::test]
async fn test_outdated_returns_empty_when_up_to_date() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    setup_installed_keg(&layout, "a", "1.0", true)?;

    let formulae = vec![make_formula("a", "1.0", &[], PLAN_SHA)];
    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(formulae, vec![], counter, layout)?;

    let entries = orchestrator.outdated(&[]).await?;
    assert!(entries.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_search_uses_metadata_store() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    // Populate metadata store with a fresh timestamp so it won't be stale.
    let store = MetadataStore::new(layout.cache_dir());
    let meta = brewdock_formula::IndexMetadata {
        etag: None,
        fetched_at: unix_now().as_secs(),
        formula_count: 3,
    };
    store.save_index(
        &[
            make_formula("jq", "1.7", &[], PLAN_SHA),
            make_formula("jql", "7.0", &[], PLAN_SHA),
            make_formula("wget", "1.24", &[], PLAN_SHA),
        ],
        &meta,
    )?;

    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(vec![], vec![], counter, layout)?;

    let results = orchestrator.search("jq").await?;
    assert_eq!(results, vec!["jq", "jql"]);

    let results = orchestrator.search("nonexistent").await?;
    assert!(results.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_search_auto_fetches_when_cache_empty() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    // No metadata cache populated, but MockRepo has formulae.
    let formulae = vec![
        make_formula("jq", "1.7", &[], PLAN_SHA),
        make_formula("wget", "1.24", &[], PLAN_SHA),
    ];
    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(formulae, vec![], counter, layout.clone())?;

    // Should auto-fetch and populate cache, then find jq.
    let results = orchestrator.search("jq").await?;
    assert_eq!(results, vec!["jq"]);

    // Cache should now be populated.
    let store = MetadataStore::new(layout.cache_dir());
    assert_eq!(store.formula_count()?, 2);
    Ok(())
}

#[tokio::test]
async fn test_info_returns_formula_details() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    setup_installed_keg(&layout, "a", "1.0", true)?;

    let formulae = vec![make_formula("a", "1.0", &["b"], PLAN_SHA)];
    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(formulae, vec![], counter, layout)?;

    let info = orchestrator.info("a").await?;
    assert_eq!(info.name, "a");
    assert_eq!(info.version, "1.0");
    assert_eq!(info.dependencies, vec!["b"]);
    assert!(info.bottle_available);
    assert_eq!(info.installed_version, Some("1.0".to_owned()));
    Ok(())
}

#[tokio::test]
async fn test_info_not_installed() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let formulae = vec![make_formula("a", "1.0", &[], PLAN_SHA)];
    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(formulae, vec![], counter, layout)?;

    let info = orchestrator.info("a").await?;
    assert!(info.installed_version.is_none());
    Ok(())
}

#[test]
fn test_list_returns_installed_kegs() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    setup_installed_keg(&layout, "a", "1.0", true)?;
    setup_installed_keg(&layout, "b", "2.0", false)?;

    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(vec![], vec![], counter, layout)?;

    let kegs = orchestrator.list()?;
    assert_eq!(kegs.len(), 2);
    assert_eq!(kegs[0].name, "a");
    assert_eq!(kegs[1].name, "b");
    Ok(())
}

#[test]
fn test_list_returns_empty_when_nothing_installed() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(vec![], vec![], counter, layout)?;

    let kegs = orchestrator.list()?;
    assert!(kegs.is_empty());
    Ok(())
}

#[test]
fn test_doctor_warns_when_no_metadata_cache() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(vec![], vec![], counter, layout)?;

    let diagnostics = orchestrator.doctor()?;
    assert!(
        diagnostics
            .iter()
            .any(|d| d.category == DiagnosticCategory::Warning
                && d.message.contains("no formula index cached"))
    );
    Ok(())
}

#[test]
fn test_doctor_detects_broken_opt_symlinks() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    // Create a broken opt symlink
    let opt_dir = layout.opt_dir();
    std::fs::create_dir_all(&opt_dir)?;
    std::os::unix::fs::symlink("/nonexistent/path", opt_dir.join("broken"))?;

    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(vec![], vec![], counter, layout)?;

    let diagnostics = orchestrator.doctor()?;
    assert!(
        diagnostics
            .iter()
            .any(|d| d.category == DiagnosticCategory::Warning
                && d.message.contains("broken symlink"))
    );
    Ok(())
}

#[test]
fn test_cleanup_nothing_to_clean() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(vec![], vec![], counter, layout)?;

    let result = orchestrator.cleanup(false)?;
    assert_eq!(result.blobs_removed, 0);
    assert_eq!(result.stores_removed, 0);
    Ok(())
}

#[test]
fn test_cleanup_removes_orphan_blobs() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    // Create an orphan blob (not referenced by any installed formula)
    let blob_dir = layout.blob_dir();
    let orphan_path = blob_dir.join("ab").join("abcdef1234567890");
    std::fs::create_dir_all(orphan_path.parent().ok_or("no parent")?)?;
    std::fs::write(&orphan_path, b"orphan data")?;

    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(vec![], vec![], counter, layout)?;

    let result = orchestrator.cleanup(false)?;
    assert!(result.blobs_removed > 0);
    assert!(!orphan_path.exists());
    Ok(())
}

#[test]
fn test_cleanup_dry_run_does_not_delete() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let blob_dir = layout.blob_dir();
    let orphan_path = blob_dir.join("ab").join("abcdef1234567890");
    std::fs::create_dir_all(orphan_path.parent().ok_or("no parent")?)?;
    std::fs::write(&orphan_path, b"orphan data")?;

    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(vec![], vec![], counter, layout)?;

    let result = orchestrator.cleanup(true)?;
    assert!(result.blobs_removed > 0);
    assert!(orphan_path.exists(), "dry run should not delete files");
    Ok(())
}
