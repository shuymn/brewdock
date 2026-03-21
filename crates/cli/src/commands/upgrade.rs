use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::{Verbosity, output};

/// Runs the upgrade command.
///
/// # Errors
///
/// Returns an error if the upgrade orchestration fails.
pub async fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    formulae: &[String],
    dry_run: bool,
    verbosity: Verbosity,
) -> Result<()> {
    let formula_names: Vec<&str> = formulae.iter().map(String::as_str).collect();

    if dry_run {
        let plan = orchestrator
            .plan_upgrade(&formula_names)
            .await
            .context("upgrade planning failed")?;
        if !verbosity.is_quiet() {
            print!("{}", output::render_upgrade_plan(&plan));
        }
        return Ok(());
    }

    let upgraded = orchestrator
        .upgrade(&formula_names)
        .await
        .context("upgrade failed")?;
    if !verbosity.is_quiet() {
        print!("{}", output::render_upgrade_summary(&upgraded));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicUsize};

    use brewdock_core::{InstallMethod, Layout, SourceBuildPlan, UpgradePlanEntry};

    use super::*;
    use crate::{
        output,
        testutil::{
            SHA_A, SHA_C, create_bottle_tar_gz, make_formula, make_orchestrator,
            setup_installed_keg,
        },
    };

    #[tokio::test]
    async fn test_commands_upgrade_installs_new_version() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        // Pre-populate filesystem state with old version.
        setup_installed_keg(&layout, "a", "1.0", true)?;

        let formula = make_formula("a", "2.0", &[], SHA_C);
        let tar = create_bottle_tar_gz("a", "2.0", &[("bin/a_tool", b"new")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula], vec![(SHA_C, tar)], counter, layout.clone())?;

        run(&orchestrator, &["a".to_owned()], false, Verbosity::Normal).await?;

        assert!(layout.cellar().join("a/2.0/bin/a_tool").exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_commands_upgrade_already_up_to_date() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        setup_installed_keg(&layout, "a", "1.0", true)?;

        let formula = make_formula("a", "1.0", &[], SHA_A);
        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout.clone())?;

        // Should succeed without error (nothing to upgrade).
        run(&orchestrator, &["a".to_owned()], false, Verbosity::Normal).await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_dry_run_upgrade_does_not_execute() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        // Pre-populate filesystem state with old version.
        setup_installed_keg(&layout, "a", "1.0", true)?;

        let formula = make_formula("a", "2.0", &[], SHA_C);
        let tar = create_bottle_tar_gz("a", "2.0", &[("bin/a_tool", b"new")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula],
            vec![(SHA_C, tar)],
            counter.clone(),
            layout.clone(),
        )?;

        run(&orchestrator, &["a".to_owned()], true, Verbosity::Normal).await?;

        // dry_run: new version not installed
        assert!(!layout.cellar().join("a/2.0").exists());
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "no downloads in dry-run"
        );

        // Old filesystem state preserved
        let keg = brewdock_cellar::find_installed_keg("a", &layout.cellar(), &layout.opt_dir())?
            .ok_or("expected keg")?;
        assert_eq!(keg.pkg_version, "1.0");
        Ok(())
    }

    #[test]
    fn test_render_upgrade_plan_includes_method() {
        let plan = vec![UpgradePlanEntry {
            name: "a".to_owned(),
            from_version: "1.0".to_owned(),
            to_version: "2.0".to_owned(),
            method: InstallMethod::Source(SourceBuildPlan {
                formula_name: "a".to_owned(),
                version: "2.0".to_owned(),
                source_url: "https://example.com/a-2.0.tar.gz".to_owned(),
                source_checksum: Some(SHA_C.to_owned()),
                build_dependencies: Vec::new(),
                runtime_dependencies: Vec::new(),
                prefix: std::path::PathBuf::from("/opt/homebrew"),
                cellar_path: std::path::PathBuf::from("/opt/homebrew/Cellar/a/2.0"),
            }),
        }];

        let rendered = output::render_upgrade_plan(&plan);
        assert!(rendered.contains("a 1.0 -> 2.0 [source]"));
    }
}
