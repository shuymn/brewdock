use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

/// Runs the upgrade command.
///
/// # Errors
///
/// Returns an error if the upgrade orchestration fails.
pub async fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    formulae: &[String],
) -> Result<()> {
    let upgraded = orchestrator
        .upgrade(formulae)
        .await
        .context("upgrade failed")?;
    if upgraded.is_empty() {
        println!("Already up-to-date");
    } else {
        for name in &upgraded {
            println!("Upgraded {name}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicUsize};

    use brewdock_cellar::{InstallRecord, StateDb};
    use brewdock_core::Layout;

    use super::*;
    use crate::testutil::{create_bottle_tar_gz, make_formula, make_orchestrator};

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

        let sha = "sha_new";
        let formula = make_formula("a", "2.0", &[], sha);
        let tar = create_bottle_tar_gz("a", "2.0", &[("bin/a_tool", b"new")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula], vec![(sha, tar)], counter, layout.clone());

        run(&orchestrator, &["a".to_owned()]).await?;

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

        let formula = make_formula("a", "1.0", &[], "sha_a");
        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout.clone());

        // Should succeed without error (nothing to upgrade).
        run(&orchestrator, &["a".to_owned()]).await?;
        Ok(())
    }
}
