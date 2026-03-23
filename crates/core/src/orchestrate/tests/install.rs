use super::*;

#[tokio::test]
async fn test_install_topological_order() -> Result<(), Box<dyn std::error::Error>> {
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

    assert_eq!(installed, vec!["b", "a"]);
    assert!(layout.cellar().join("a/1.0/bin/a_tool").exists());
    assert!(layout.cellar().join("b/2.0/bin/b_tool").exists());
    assert!(layout.prefix().join("bin/a_tool").is_symlink());
    assert!(layout.prefix().join("bin/b_tool").is_symlink());
    assert!(layout.opt_dir().join("a").is_symlink());
    assert!(layout.opt_dir().join("b").is_symlink());
    assert!(layout.cellar().join("a/1.0/INSTALL_RECEIPT.json").exists());
    assert!(layout.cellar().join("b/2.0/INSTALL_RECEIPT.json").exists());

    let keg_a =
        find_installed_keg("a", &layout.cellar(), &layout.opt_dir())?.ok_or("expected a")?;
    assert_eq!(keg_a.pkg_version, "1.0");
    assert!(keg_a.installed_on_request);
    let keg_b =
        find_installed_keg("b", &layout.cellar(), &layout.opt_dir())?.ok_or("expected b")?;
    assert_eq!(keg_b.pkg_version, "2.0");
    assert!(!keg_b.installed_on_request);

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

    setup_installed_keg(&layout, "b", "2.0", false)?;

    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(
        vec![formula_a, formula_b],
        vec![(sha_a, tar_a), (sha_b, tar_b)],
        counter.clone(),
        layout.clone(),
    )?;

    let installed = orchestrator.install(&["a"]).await?;
    assert_eq!(installed, vec!["a"]);
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn test_plan_install_uses_compatible_bottle() -> Result<(), Box<dyn std::error::Error>> {
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
async fn test_plan_install_uses_source_when_bottle_missing()
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
    let orchestrator = make_orchestrator(vec![formula, pkgconf], vec![], counter, layout.clone())?;

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
async fn test_plan_install_keeps_post_install_plannable() -> Result<(), Box<dyn std::error::Error>>
{
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
async fn test_build_execution_plan_marks_boundaries() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let bottle_formula = make_formula("bottle", "1.0", &[], PLAN_SHA);
    let mut source_formula = make_formula("source", "2.0", &[], SHA_B);
    source_formula.versions.bottle = false;
    source_formula.bottle.stable = None;

    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(
        vec![bottle_formula.clone(), source_formula.clone()],
        vec![],
        counter,
        layout,
    )?;

    let mut cache = FormulaCache::new();
    cache.insert(bottle_formula);
    cache.insert(source_formula);

    let plan =
        orchestrator.build_execution_plan(&["bottle".to_owned(), "source".to_owned()], &cache)?;

    assert_eq!(
        plan.acquire_concurrency,
        Orchestrator::<crate::testutil::MockRepo, crate::testutil::MockDownloader>::MAX_ACQUIRE_CONCURRENCY
    );
    assert_eq!(
        plan.entries
            .iter()
            .map(|entry| entry.formula.name.as_str())
            .collect::<Vec<_>>(),
        vec!["bottle", "source"]
    );
    assert!(matches!(
        plan.entries[0].finalize,
        FinalizeStep::FinalizeBottle
    ));
    assert!(matches!(
        plan.entries[1].finalize,
        FinalizeStep::BuildFromSource
    ));
    Ok(())
}

#[tokio::test]
async fn test_plan_upgrade_reuses_method_resolution() -> Result<(), Box<dyn std::error::Error>> {
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
async fn test_install_rejects_unsupported_before_download() -> Result<(), Box<dyn std::error::Error>>
{
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
    let orchestrator = make_orchestrator(vec![formula], vec![], counter.clone(), layout.clone())?;

    let result = orchestrator.install(&["disabled_pkg"]).await;

    assert!(result.is_err());
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    assert!(!layout.cellar().join("disabled_pkg").exists());
    Ok(())
}

#[tokio::test]
async fn test_install_any_skip_relocation_bottle_succeeds() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = "abababababababababababababababababababababababababababababababab";
    let mut formula = make_formula("skip-reloc", "1.0", &[], sha);
    set_bottle_cellar(&mut formula, CellarType::AnySkipRelocation);

    let tar = create_bottle_tar_gz("skip-reloc", "1.0", &[("bin/tool", b"#!/bin/sh\necho ok")])?;

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
async fn test_any_skip_relocation_bottle_relocates_placeholders()
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

    let installed_shebang = std::fs::read_to_string(layout.cellar().join("pytools/3.0/bin/tool"))?;
    assert_eq!(
        installed_shebang,
        format!("#!{}/bin/python3\nimport sys\n", layout.prefix().display(),),
    );

    let installed_pth =
        std::fs::read_to_string(layout.cellar().join("pytools/3.0/lib/python3/site.pth"))?;
    assert_eq!(
        installed_pth,
        format!(
            "{}/Cellar/pytools/3.0/lib/python3\n",
            layout.prefix().display()
        ),
    );

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
    assert_eq!(download_count.load(Ordering::SeqCst), 0);

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

#[tokio::test]
async fn test_install_independent_formulae() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let sha_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let sha_c = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let formula_a = make_formula("alpha", "1.0", &[], sha_a);
    let formula_b = make_formula("bravo", "2.0", &[], sha_b);
    let formula_c = make_formula("charlie", "3.0", &[], sha_c);

    let tar_a = create_bottle_tar_gz("alpha", "1.0", &[("bin/alpha_tool", b"#!/bin/sh\necho a")])?;
    let tar_b = create_bottle_tar_gz("bravo", "2.0", &[("bin/bravo_tool", b"#!/bin/sh\necho b")])?;
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
async fn test_install_finalize_failure_preserves_installs() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let sha_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let sha_c = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let formula_a = make_formula("alpha", "1.0", &[], sha_a);
    let formula_b = make_formula("bravo", "2.0", &[], sha_b);
    let formula_c = make_formula("charlie", "3.0", &[], sha_c);

    let tar_a = create_bottle_tar_gz("alpha", "1.0", &[("bin/tool", b"#!/bin/sh\necho a")])?;
    let tar_b = create_bottle_tar_gz("bravo", "2.0", &[("bin/bravo_tool", b"#!/bin/sh\necho b")])?;
    let tar_c = create_bottle_tar_gz("charlie", "3.0", &[("bin/tool", b"#!/bin/sh\necho c")])?;

    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(
        vec![formula_a, formula_b, formula_c],
        vec![(sha_a, tar_a), (sha_b, tar_b), (sha_c, tar_c)],
        counter.clone(),
        layout.clone(),
    )?;

    let result = orchestrator.install(&["alpha", "bravo", "charlie"]).await;

    assert!(result.is_err());
    assert_installed(&layout, "alpha");
    assert_installed(&layout, "bravo");
    assert_not_installed(&layout, "charlie");
    Ok(())
}

#[tokio::test]
async fn test_install_bounds_prefetch_downloads() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let formulas = vec![
        make_formula("alpha", "1.0", &[], SHA_A),
        make_formula("bravo", "1.0", &[], SHA_B),
        make_formula("charlie", "1.0", &[], SHA_C),
        make_formula(
            "delta",
            "1.0",
            &[],
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        ),
        make_formula(
            "echo",
            "1.0",
            &[],
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        ),
        make_formula(
            "foxtrot",
            "1.0",
            &[],
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        ),
    ];
    let payloads = vec![
        (
            SHA_A,
            create_bottle_tar_gz("alpha", "1.0", &[("bin/alpha-tool", b"#!/bin/sh\n")])?,
        ),
        (
            SHA_B,
            create_bottle_tar_gz("bravo", "1.0", &[("bin/bravo-tool", b"#!/bin/sh\n")])?,
        ),
        (
            SHA_C,
            create_bottle_tar_gz("charlie", "1.0", &[("bin/charlie-tool", b"#!/bin/sh\n")])?,
        ),
        (
            "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            create_bottle_tar_gz("delta", "1.0", &[("bin/delta-tool", b"#!/bin/sh\n")])?,
        ),
        (
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            create_bottle_tar_gz("echo", "1.0", &[("bin/echo-tool", b"#!/bin/sh\n")])?,
        ),
        (
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            create_bottle_tar_gz("foxtrot", "1.0", &[("bin/foxtrot-tool", b"#!/bin/sh\n")])?,
        ),
    ];
    let downloader = TrackingDownloader::new(payloads, Duration::from_millis(30));
    let host_tag: HostTag = HOST_TAG.parse()?;
    let orchestrator = Orchestrator::new(MockRepo::new(formulas), downloader, layout, host_tag);

    let installed = orchestrator
        .install(&["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"])
        .await?;

    assert_eq!(installed.len(), 6);
    assert_eq!(
        orchestrator.downloader.max_in_flight(),
        Orchestrator::<MockRepo, TrackingDownloader>::MAX_ACQUIRE_CONCURRENCY,
        "acquire should cap concurrent downloads at the execution plan limit",
    );
    Ok(())
}

#[tokio::test]
async fn test_install_download_fail_does_not_publish_blob() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha = SHA_A;
    let formula = make_formula("alpha", "1.0", &[], sha);
    let orchestrator = make_orchestrator(
        vec![formula],
        vec![],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let result = orchestrator.install(&["alpha"]).await;

    assert!(result.is_err());
    let blob_store = BlobStore::new(&layout.blob_dir());
    assert!(
        !blob_store.has(sha)?,
        "failed downloads must not publish incomplete payloads to the blob store",
    );
    Ok(())
}

#[tokio::test]
async fn test_install_skips_download_for_existing_blob() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let formula_a = make_formula("a", "1.0", &[], sha_a);
    let tar_a = create_simple_bottle("a", "1.0")?;

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
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "download should be skipped when blob exists in store"
    );
    Ok(())
}

#[tokio::test]
async fn test_install_skips_extract_for_existing_dir() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let formula_a = make_formula("a", "1.0", &[], sha_a);
    let tar_a = create_bottle_tar_gz("a", "1.0", &[("bin/tool", b"#!/bin/sh\necho ok\n")])?;

    let blob_store = BlobStore::new(&layout.blob_dir());
    blob_store.put(sha_a, &tar_a)?;
    let blob_path = blob_store.blob_path(sha_a)?;

    let extract_dir = layout.store_dir().join(sha_a);
    extract_tar_gz(&blob_path, &extract_dir)?;

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
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "download should be skipped when blob exists"
    );
    assert!(layout.cellar().join("a/1.0/bin/tool").exists());
    Ok(())
}

#[tokio::test]
async fn test_install_warm_path_multiple_formulae() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let sha_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let formula_a = make_formula("a", "1.0", &[], sha_a);
    let formula_b = make_formula("b", "2.0", &[], sha_b);

    let tar_a = create_bottle_tar_gz("a", "1.0", &[("bin/a_tool", b"#!/bin/sh\necho a")])?;
    let tar_b = create_bottle_tar_gz("b", "2.0", &[("bin/b_tool", b"#!/bin/sh\necho b")])?;

    let blob_store = BlobStore::new(&layout.blob_dir());
    blob_store.put(sha_a, &tar_a)?;
    blob_store.put(sha_b, &tar_b)?;

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
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn test_install_download_fail_prevents_prefix_mutation()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let sha_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let sha_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let sha_c = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let formula_a = make_formula("alpha", "1.0", &[], sha_a);
    let formula_b = make_formula("bravo", "2.0", &[], sha_b);
    let formula_c = make_formula("charlie", "3.0", &[], sha_c);

    let tar_a = create_bottle_tar_gz("alpha", "1.0", &[("bin/alpha_tool", b"#!/bin/sh\necho a")])?;
    let tar_b = create_bottle_tar_gz("bravo", "2.0", &[("bin/bravo_tool", b"#!/bin/sh\necho b")])?;

    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(
        vec![formula_a, formula_b, formula_c],
        vec![(sha_a, tar_a), (sha_b, tar_b)],
        counter.clone(),
        layout.clone(),
    )?;

    let result = orchestrator.install(&["alpha", "bravo", "charlie"]).await;

    assert!(result.is_err());
    assert_not_installed(&layout, "alpha");
    assert_not_installed(&layout, "bravo");
    assert_not_installed(&layout, "charlie");
    Ok(())
}
