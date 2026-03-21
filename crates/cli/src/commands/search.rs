use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::Verbosity;

/// Runs the search command.
///
/// # Errors
///
/// Returns an error if the search fails.
pub async fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    pattern: &str,
    verbosity: Verbosity,
) -> Result<()> {
    let results = orchestrator
        .search(pattern)
        .await
        .context("search failed")?;

    if !verbosity.is_quiet() {
        if results.is_empty() {
            println!("No formulae found matching \"{pattern}\"");
        } else {
            for name in &results {
                println!("{name}");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicUsize};

    use brewdock_core::Layout;

    use super::*;
    use crate::testutil::{SHA_A, make_formula, make_orchestrator};

    #[tokio::test]
    async fn test_search_auto_fetches_on_cache_miss() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        // MockRepo has formulae but SQLite cache is empty.
        let formulae = vec![
            make_formula("jq", "1.7", &[], SHA_A),
            make_formula("wget", "1.24", &[], SHA_A),
        ];
        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(formulae, vec![], counter, layout)?;

        // Should auto-fetch and find jq.
        run(&orchestrator, "jq", Verbosity::Normal).await?;
        Ok(())
    }
}
