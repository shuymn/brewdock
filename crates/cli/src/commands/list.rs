use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::Verbosity;

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
        if kegs.is_empty() {
            println!("No formulae installed");
        } else {
            for keg in &kegs {
                println!("{} {}", keg.name, keg.pkg_version);
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
    use crate::testutil::{make_orchestrator, setup_installed_keg};

    #[test]
    fn test_list_shows_installed_formulae() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        setup_installed_keg(&layout, "jq", "1.7", true)?;
        setup_installed_keg(&layout, "wget", "1.24", true)?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![], vec![], counter, layout)?;

        run(&orchestrator, Verbosity::Normal)?;
        Ok(())
    }

    #[test]
    fn test_list_shows_empty_when_nothing_installed() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![], vec![], counter, layout)?;

        run(&orchestrator, Verbosity::Normal)?;
        Ok(())
    }
}
