use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::{Verbosity, output};

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
        print!("{}", output::render_search_results(pattern, &results));
    }

    Ok(())
}
