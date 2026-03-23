use super::*;

#[tokio::test]
async fn test_upgrade_unlinks_old_and_installs_new() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let old_keg = layout.cellar().join("a/1.0");
    std::fs::create_dir_all(old_keg.join("bin"))?;
    std::fs::write(old_keg.join("bin/a_tool"), "old_version")?;
    link(&old_keg, layout.prefix())?;

    assert_eq!(
        std::fs::read_to_string(layout.prefix().join("bin/a_tool"))?,
        "old_version"
    );

    write_receipt(
        &old_keg,
        &InstallReceipt::for_bottle(
            InstallReason::OnRequest,
            Some(1_000.0),
            Vec::new(),
            ReceiptSource {
                path: String::new(),
                tap: "homebrew/core".to_owned(),
                spec: "stable".to_owned(),
                versions: ReceiptSourceVersions {
                    stable: "1.0".to_owned(),
                    head: None,
                    version_scheme: 0,
                },
            },
        ),
    )?;
    std::fs::create_dir_all(layout.opt_dir())?;
    atomic_symlink_replace(&old_keg, &layout.opt_dir().join("a"))?;

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
    assert_eq!(
        std::fs::read_to_string(layout.cellar().join("a/2.0/bin/a_tool"))?,
        "new_version"
    );
    assert!(layout.prefix().join("bin/a_tool").is_symlink());
    assert_eq!(
        std::fs::read_to_string(layout.prefix().join("bin/a_tool"))?,
        "new_version"
    );

    let keg =
        find_installed_keg("a", &layout.cellar(), &layout.opt_dir())?.ok_or("expected keg")?;
    assert_eq!(keg.pkg_version, "2.0");
    assert!(keg.installed_on_request);
    Ok(())
}

#[tokio::test]
async fn test_upgrade_skips_current_version() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    setup_installed_keg(&layout, "a", "1.0", true)?;

    let formula = make_formula(
        "a",
        "1.0",
        &[],
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
    );
    let counter = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(vec![formula], vec![], counter.clone(), layout.clone())?;

    let upgraded = orchestrator.upgrade(&["a"]).await?;

    assert!(upgraded.is_empty());
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn test_upgrade_restores_old_links_on_download_fail() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());
    setup_installed_keg(&layout, "demo", "1.0", true)?;
    let old_keg = layout.cellar().join("demo/1.0");
    std::fs::create_dir_all(old_keg.join("bin"))?;
    std::fs::create_dir_all(layout.prefix().join("bin"))?;
    std::fs::write(old_keg.join("bin/tool"), "#!/bin/sh\necho old")?;
    std::os::unix::fs::symlink(
        "../Cellar/demo/1.0/bin/tool",
        layout.prefix().join("bin/tool"),
    )?;

    let sha = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let formula = make_formula("demo", "2.0", &[], sha);
    let download_count = Arc::new(AtomicUsize::new(0));
    let orchestrator = make_orchestrator(
        vec![formula],
        vec![],
        download_count.clone(),
        layout.clone(),
    )?;

    let result = orchestrator.upgrade(&["demo"]).await;

    assert!(matches!(
        result,
        Err(BrewdockError::Bottle(_) | BrewdockError::Cellar(_) | BrewdockError::Io(_))
    ));
    assert!(layout.cellar().join("demo/1.0/bin/tool").exists());
    let opt_link = layout.opt_dir().join("demo");
    assert!(opt_link.is_symlink());
    assert_eq!(
        std::fs::read_link(&opt_link)?,
        layout.cellar().join("demo/1.0")
    );
    assert!(layout.prefix().join("bin/tool").is_symlink());
    assert_eq!(download_count.load(Ordering::SeqCst), 1);
    Ok(())
}
