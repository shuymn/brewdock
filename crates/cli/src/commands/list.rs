use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::{Verbosity, output};

/// Runs the list command.
///
/// # Errors
///
/// Returns an error if listing installed formulae fails.
pub fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    verbosity: Verbosity,
) -> Result<()> {
    let kegs = orchestrator
        .list()
        .context("listing installed formulae failed")?;

    if !verbosity.is_quiet() {
        print!("{}", output::render_list(&kegs));
    }

    Ok(())
}
