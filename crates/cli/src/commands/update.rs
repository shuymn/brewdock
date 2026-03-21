use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::{Verbosity, output};

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
            print!("{}", output::render_update_dry_run());
        }
        return Ok(());
    }

    let count = orchestrator.update().await.context("update failed")?;
    if !verbosity.is_quiet() {
        print!("{}", output::render_update_summary(count));
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

        let db_path = layout.cache_dir().join("formula.db");
        assert!(db_path.exists(), "SQLite database should be written");

        let store = brewdock_formula::MetadataStore::new(layout.cache_dir());
        assert_eq!(store.formula_count()?, 2);

        let meta = store.load_metadata()?.ok_or("metadata should exist")?;
        assert!(meta.fetched_at > 0, "fetched_at should be set");
        assert_eq!(meta.formula_count, 2, "formula_count should match");
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

        let db_path = layout.cache_dir().join("formula.db");
        assert!(!db_path.exists(), "dry-run should not cache");
        Ok(())
    }
}
