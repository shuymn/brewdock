use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::Verbosity;

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
        if entries.is_empty() {
            println!("All formulae are up to date");
        } else {
            for entry in &entries {
                println!(
                    "{} {} -> {}",
                    entry.name, entry.current_version, entry.latest_version
                );
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
    use crate::testutil::{SHA_A, SHA_C, make_formula, make_orchestrator, setup_installed_keg};

    #[tokio::test]
    async fn test_outdated_shows_outdated_formula() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        setup_installed_keg(&layout, "a", "1.0", true)?;

        let formula = make_formula("a", "2.0", &[], SHA_C);
        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout)?;

        run(&orchestrator, &["a".to_owned()], Verbosity::Normal).await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_outdated_shows_up_to_date() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        setup_installed_keg(&layout, "a", "1.0", true)?;

        let formula = make_formula("a", "1.0", &[], SHA_A);
        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout)?;

        run(&orchestrator, &["a".to_owned()], Verbosity::Normal).await?;
        Ok(())
    }
}
