use std::sync::{Arc, atomic::AtomicUsize};

use brewdock_cellar::find_installed_keg;
use brewdock_formula::{Formula, NamedEntry, Requirement};

use super::{BrewdockError, InstallMethod, Layout};
use crate::{
    error::SourceBuildError,
    testutil::{
        PLAN_SHA, assert_installed, assert_not_installed, create_bottle_tar_gz,
        create_source_tar_gz, create_source_tar_gz_with_raw_paths, make_formula, make_orchestrator,
        setup_installed_keg,
    },
};

#[tokio::test]
async fn test_source_fallback_installs_build_deps_before_target()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    let helper_runtime_sha = "5656565656565656565656565656565656565656565656565656565656565656";
    let build_helper_sha = "6767676767676767676767676767676767676767676767676767676767676767";
    let runtime_dep_sha = "7878787878787878787878787878787878787878787878787878787878787878";
    let source_sha = "8989898989898989898989898989898989898989898989898989898989898989";

    let helper_runtime = make_formula("helper-runtime", "1.0", &[], helper_runtime_sha);
    let build_helper = make_formula("build-helper", "1.0", &["helper-runtime"], build_helper_sha);
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
    assert_installed(&layout, "target");
    Ok(())
}

fn make_no_bottle_formula_with_requirements(requirements: Vec<Requirement>) -> Formula {
    let mut formula = make_formula("a", "1.0", &[], PLAN_SHA);
    formula.versions.bottle = false;
    formula.bottle.stable = None;
    formula.requirements = requirements;
    formula
}

#[tokio::test]
async fn test_source_fallback_allows_system_requirement() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());
    let formula =
        make_no_bottle_formula_with_requirements(vec![Requirement::Name("xcode".to_owned())]);

    let orchestrator = make_orchestrator(
        vec![formula],
        vec![],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;
    let plan = orchestrator.plan_install(&["a"]).await?;
    let entry = plan.first().ok_or("expected install plan entry")?;

    assert!(matches!(
        &entry.method,
        InstallMethod::Source(source)
            if source.formula_name == "a" && source.cellar_path == layout.cellar().join("a/1.0")
    ));
    Ok(())
}

#[tokio::test]
async fn test_source_fallback_rejects_unsupported_requirement()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());
    let formula = make_no_bottle_formula_with_requirements(vec![Requirement::Name(
        "some_custom_requirement".to_owned(),
    )]);

    let orchestrator =
        make_orchestrator(vec![formula], vec![], Arc::new(AtomicUsize::new(0)), layout)?;
    let result = orchestrator.plan_install(&["a"]).await;

    assert!(matches!(
        result,
        Err(BrewdockError::SourceBuild(
            SourceBuildError::UnsupportedRequirement(requirement)
        )) if requirement == "some_custom_requirement"
    ));
    Ok(())
}

#[tokio::test]
async fn test_source_fallback_rejects_future_macos_requirement()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());
    let formula =
        make_no_bottle_formula_with_requirements(vec![Requirement::Detailed(NamedEntry {
            name: "macos".to_owned(),
            version: Some("26".to_owned()),
            contexts: Vec::new(),
            specs: Vec::new(),
        })]);

    let orchestrator =
        make_orchestrator(vec![formula], vec![], Arc::new(AtomicUsize::new(0)), layout)?;
    let result = orchestrator.plan_install(&["a"]).await;

    assert!(matches!(
        result,
        Err(BrewdockError::SourceBuild(
            SourceBuildError::UnsupportedRequirement(requirement)
        )) if requirement == "macos"
    ));
    Ok(())
}

#[tokio::test]
async fn test_source_fallback_rejects_wrong_arch_requirement()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());
    let formula =
        make_no_bottle_formula_with_requirements(vec![Requirement::Detailed(NamedEntry {
            name: "arch".to_owned(),
            version: Some("x86_64".to_owned()),
            contexts: Vec::new(),
            specs: Vec::new(),
        })]);

    let orchestrator =
        make_orchestrator(vec![formula], vec![], Arc::new(AtomicUsize::new(0)), layout)?;
    let result = orchestrator.plan_install(&["a"]).await;

    assert!(matches!(
        result,
        Err(BrewdockError::SourceBuild(
            SourceBuildError::UnsupportedRequirement(requirement)
        )) if requirement == "arch"
    ));
    Ok(())
}

#[tokio::test]
async fn test_source_fallback_allows_maximum_macos_requirement()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());
    let formula =
        make_no_bottle_formula_with_requirements(vec![Requirement::Detailed(NamedEntry {
            name: "maximum_macos".to_owned(),
            version: Some("26".to_owned()),
            contexts: Vec::new(),
            specs: Vec::new(),
        })]);

    let orchestrator = make_orchestrator(
        vec![formula],
        vec![],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;
    let plan = orchestrator.plan_install(&["a"]).await?;
    let entry = plan.first().ok_or("expected install plan entry")?;

    assert!(matches!(
        &entry.method,
        InstallMethod::Source(source)
            if source.formula_name == "a" && source.cellar_path == layout.cellar().join("a/1.0")
    ));
    Ok(())
}

#[tokio::test]
async fn test_source_fallback_cleans_up_failed_build() -> Result<(), Box<dyn std::error::Error>> {
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
    assert_not_installed(&layout, "broken");
    Ok(())
}

#[tokio::test]
async fn test_source_fallback_plan_upgrade_reuses_source_resolution()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    setup_installed_keg(&layout, "portable", "1.0", true)?;

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
async fn test_source_fallback_upgrade_installs_build_deps() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());

    setup_installed_keg(&layout, "target", "1.0", true)?;

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
    assert_installed(&layout, "build-helper");
    let target_keg = find_installed_keg("target", &layout.cellar(), &layout.opt_dir())?
        .ok_or("expected upgraded target record")?;
    assert_eq!(target_keg.pkg_version, "2.0");
    Ok(())
}

#[tokio::test]
async fn test_source_archive_traversal() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let layout = Layout::with_root(dir.path());
    let sha = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    let mut formula = make_formula("evil", "1.0", &[], sha);
    formula.bottle.stable = None;
    let archive = create_source_tar_gz_with_raw_paths(&[
        (b"evil-1.0/README.md", b"legit source tree"),
        (b"../../escape.txt", b"escaped"),
    ])?;
    let orchestrator = make_orchestrator(
        vec![formula],
        vec![(sha, archive)],
        Arc::new(AtomicUsize::new(0)),
        layout.clone(),
    )?;

    let result = orchestrator.install(&["evil"]).await;

    assert!(
        result.is_err(),
        "malformed source archive should fail closed"
    );
    assert!(
        !layout.cache_dir().join("sources/escape.txt").exists(),
        "path traversal must not escape the temporary source root"
    );
    assert!(!layout.cellar().join("evil/1.0").exists());
    assert!(!layout.opt_dir().join("evil").exists());
    Ok(())
}
