use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

/// Runs the update command.
///
/// # Errors
///
/// Returns an error if fetching or caching the formula index fails.
pub async fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
) -> Result<()> {
    let count = orchestrator.update().await.context("update failed")?;
    println!("Updated {count} formulae");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicUsize};

    use brewdock_core::Layout;

    use super::*;
    use crate::testutil::{make_formula, make_orchestrator};

    #[tokio::test]
    async fn test_commands_update_caches_index() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let formula_a = make_formula("a", "1.0", &[], "sha_a");
        let formula_b = make_formula("b", "2.0", &[], "sha_b");

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula_a, formula_b], vec![], counter, layout.clone());

        run(&orchestrator).await?;

        let cache_path = layout.cache_dir().join("formula.json");
        assert!(cache_path.exists());

        let data = std::fs::read_to_string(&cache_path)?;
        let cached: Vec<brewdock_formula::Formula> = serde_json::from_str(&data)?;
        assert_eq!(cached.len(), 2);
        Ok(())
    }
}
