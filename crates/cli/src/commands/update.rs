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
