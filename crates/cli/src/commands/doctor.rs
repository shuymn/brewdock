use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::{Verbosity, output};

/// Runs the doctor command.
///
/// # Errors
///
/// Returns an error if diagnostics fail.
pub fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    verbosity: Verbosity,
) -> Result<()> {
    let diagnostics = orchestrator.doctor().context("doctor check failed")?;

    if !verbosity.is_quiet() {
        print!("{}", output::render_doctor(&diagnostics));
    }

    Ok(())
}
