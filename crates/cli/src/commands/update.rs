use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::Verbosity;

/// Runs the update command.
///
/// # Errors
///
/// Returns an error if fetching or caching the formula index fails.
pub async fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    dry_run: bool,
    verbosity: Verbosity,
) -> Result<()> {
    if dry_run {
        if !verbosity.is_quiet() {
            println!("Would update formula index");
        }
        return Ok(());
    }

    let count = orchestrator.update().await.context("update failed")?;
    if !verbosity.is_quiet() {
        println!("Updated {count} formulae");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicUsize};

    use brewdock_core::Layout;

    use super::*;
    use crate::testutil::{SHA_A, SHA_B, make_formula, make_orchestrator};

    #[tokio::test]
    async fn test_commands_update_caches_index() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let formula_a = make_formula("a", "1.0", &[], SHA_A);
        let formula_b = make_formula("b", "2.0", &[], SHA_B);

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula_a, formula_b], vec![], counter, layout.clone())?;

        run(&orchestrator, false, Verbosity::Normal).await?;

        let cache_path = layout.cache_dir().join("formula.json");
        assert!(cache_path.exists());

        let data = std::fs::read_to_string(&cache_path)?;
        let cached: Vec<brewdock_formula::Formula> = serde_json::from_str(&data)?;
        assert_eq!(cached.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn test_dry_run_update_does_not_cache() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let formula = make_formula("a", "1.0", &[], SHA_A);
        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout.clone())?;

        run(&orchestrator, true, Verbosity::Normal).await?;

        let cache_path = layout.cache_dir().join("formula.json");
        assert!(!cache_path.exists(), "dry-run should not cache");
        Ok(())
    }
}
