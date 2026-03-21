use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::{Verbosity, output};

/// Runs the cleanup command.
///
/// # Errors
///
/// Returns an error if cleanup fails.
pub fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    dry_run: bool,
    verbosity: Verbosity,
) -> Result<()> {
    let result = orchestrator.cleanup(dry_run).context("cleanup failed")?;

    if !verbosity.is_quiet() {
        print!("{}", output::render_cleanup(&result, dry_run));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_cleanup_output_is_covered_by_output_module() {}
}
