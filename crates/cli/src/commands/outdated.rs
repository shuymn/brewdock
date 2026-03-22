use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::{Verbosity, output};

/// Runs the outdated command.
///
/// # Errors
///
/// Returns an error if the outdated check fails.
pub async fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    formulae: &[String],
    verbosity: Verbosity,
) -> Result<()> {
    let formula_names: Vec<&str> = formulae.iter().map(String::as_str).collect();

    let entries = orchestrator
        .outdated(&formula_names)
        .await
        .context("outdated check failed")?;

    if !verbosity.is_quiet() {
        print!("{}", output::render_outdated(&entries));
    }

    Ok(())
}
