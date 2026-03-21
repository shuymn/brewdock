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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicUsize};

    use brewdock_core::Layout;

    use super::*;
    use crate::testutil::make_orchestrator;

    #[test]
    fn test_doctor_runs_without_error() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![], vec![], counter, layout)?;

        run(&orchestrator, Verbosity::Normal)?;
        Ok(())
    }
}
