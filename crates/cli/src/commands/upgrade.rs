use std::fmt::Write;

use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator, UpgradePlanEntry};

/// Runs the upgrade command.
///
/// # Errors
///
/// Returns an error if the upgrade orchestration fails.
pub async fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    formulae: &[String],
    dry_run: bool,
    quiet: bool,
) -> Result<()> {
    let formula_names: Vec<&str> = formulae.iter().map(String::as_str).collect();

    if dry_run {
        let plan = orchestrator
            .plan_upgrade(&formula_names)
            .await
            .context("upgrade planning failed")?;
        if !quiet {
            print!("{}", render_upgrade_plan(&plan));
        }
        return Ok(());
    }

    let upgraded = orchestrator
        .upgrade(&formula_names)
        .await
        .context("upgrade failed")?;
    if !quiet {
        if upgraded.is_empty() {
            println!("Already up-to-date");
        } else {
            for name in &upgraded {
                println!("Upgraded {name}");
            }
        }
    }
    Ok(())
}

fn render_upgrade_plan(plan: &[UpgradePlanEntry]) -> String {
    if plan.is_empty() {
        return "Already up-to-date\n".to_owned();
    }

    let mut output = String::from("Would upgrade:\n");
    for entry in plan {
        let _ = writeln!(
            output,
            "  {} {} -> {} [{}]",
            entry.name, entry.from_version, entry.to_version, entry.method
        );
    }
    output
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicUsize};

    use brewdock_cellar::{InstallRecord, StateDb};
    use brewdock_core::{InstallMethod, Layout, SourceBuildPlan, UpgradePlanEntry};

    use super::*;
    use crate::testutil::{SHA_A, SHA_C, create_bottle_tar_gz, make_formula, make_orchestrator};

    #[tokio::test]
    async fn test_commands_upgrade_installs_new_version() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        // Pre-populate state DB with old version.
        let state_db = StateDb::open(&layout.db_path())?;
        state_db.insert(&InstallRecord {
            name: "a".to_owned(),
            version: "1.0".to_owned(),
            revision: 0,
            installed_on_request: true,
            installed_at: "1000".to_owned(),
        })?;
        drop(state_db);

        let formula = make_formula("a", "2.0", &[], SHA_C);
        let tar = create_bottle_tar_gz("a", "2.0", &[("bin/a_tool", b"new")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula], vec![(SHA_C, tar)], counter, layout.clone())?;

        run(&orchestrator, &["a".to_owned()], false, false).await?;

        assert!(layout.cellar().join("a/2.0/bin/a_tool").exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_commands_upgrade_already_up_to_date() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let state_db = StateDb::open(&layout.db_path())?;
        state_db.insert(&InstallRecord {
            name: "a".to_owned(),
            version: "1.0".to_owned(),
            revision: 0,
            installed_on_request: true,
            installed_at: "1000".to_owned(),
        })?;
        drop(state_db);

        let formula = make_formula("a", "1.0", &[], SHA_A);
        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout.clone())?;

        // Should succeed without error (nothing to upgrade).
        run(&orchestrator, &["a".to_owned()], false, false).await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_dry_run_upgrade_does_not_execute() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        // Pre-populate state DB with old version.
        let state_db = StateDb::open(&layout.db_path())?;
        state_db.insert(&InstallRecord {
            name: "a".to_owned(),
            version: "1.0".to_owned(),
            revision: 0,
            installed_on_request: true,
            installed_at: "1000".to_owned(),
        })?;
        drop(state_db);

        let formula = make_formula("a", "2.0", &[], SHA_C);
        let tar = create_bottle_tar_gz("a", "2.0", &[("bin/a_tool", b"new")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula],
            vec![(SHA_C, tar)],
            counter.clone(),
            layout.clone(),
        )?;

        run(&orchestrator, &["a".to_owned()], true, false).await?;

        // dry_run: new version not installed
        assert!(!layout.cellar().join("a/2.0").exists());
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "no downloads in dry-run"
        );

        // Old state preserved
        let state_db = StateDb::open(&layout.db_path())?;
        let record = state_db.get("a")?.ok_or("expected record")?;
        assert_eq!(record.version, "1.0");
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

        let rendered = render_upgrade_plan(&plan);
        assert!(rendered.contains("a 1.0 -> 2.0 [source]"));
    }
}
