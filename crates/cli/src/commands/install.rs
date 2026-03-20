use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

/// Runs the install command.
///
/// # Errors
///
/// Returns an error if the install orchestration fails.
pub async fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    formulae: &[String],
    dry_run: bool,
    quiet: bool,
) -> Result<()> {
    if dry_run {
        let plan = orchestrator
            .plan_install(formulae)
            .await
            .context("install planning failed")?;
        if !quiet {
            if plan.is_empty() {
                println!("Nothing to install");
            } else {
                println!("Would install:");
                for entry in &plan {
                    println!("  {} {}", entry.name, entry.version);
                }
            }
        }
        return Ok(());
    }

    let installed = orchestrator
        .install(formulae)
        .await
        .context("install failed")?;
    if !quiet {
        for name in &installed {
            println!("Installed {name}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicUsize};

    use brewdock_core::Layout;

    use super::*;
    use crate::testutil::{create_bottle_tar_gz, make_formula, make_orchestrator};

    #[tokio::test]
    async fn test_commands_install_single_formula() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha = "sha_a";
        let formula = make_formula("a", "1.0", &[], sha);
        let tar = create_bottle_tar_gz("a", "1.0", &[("bin/a_tool", b"#!/bin/sh\necho a")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula], vec![(sha, tar)], counter, layout.clone());

        run(&orchestrator, &["a".to_owned()], false, false).await?;

        assert!(layout.cellar().join("a/1.0/bin/a_tool").exists());
        assert!(layout.prefix().join("bin/a_tool").is_symlink());
        Ok(())
    }

    #[tokio::test]
    async fn test_commands_install_with_deps() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha_a = "sha_a";
        let sha_b = "sha_b";
        let formula_a = make_formula("a", "1.0", &["b"], sha_a);
        let formula_b = make_formula("b", "2.0", &[], sha_b);

        let tar_a = create_bottle_tar_gz("a", "1.0", &[("bin/a_tool", b"#!/bin/sh")])?;
        let tar_b = create_bottle_tar_gz("b", "2.0", &[("bin/b_tool", b"#!/bin/sh")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula_a, formula_b],
            vec![(sha_a, tar_a), (sha_b, tar_b)],
            counter,
            layout.clone(),
        );

        run(&orchestrator, &["a".to_owned()], false, false).await?;

        assert!(layout.cellar().join("a/1.0/bin/a_tool").exists());
        assert!(layout.cellar().join("b/2.0/bin/b_tool").exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_dry_run_install_does_not_execute() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha = "sha_a";
        let formula = make_formula("a", "1.0", &[], sha);
        let tar = create_bottle_tar_gz("a", "1.0", &[("bin/a_tool", b"#!/bin/sh")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula],
            vec![(sha, tar)],
            counter.clone(),
            layout.clone(),
        );

        run(&orchestrator, &["a".to_owned()], true, false).await?;

        // dry_run: no files created, no downloads attempted
        assert!(!layout.cellar().join("a").exists());
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "no downloads in dry-run"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_dry_run_install_with_deps() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let sha_a = "sha_a";
        let sha_b = "sha_b";
        let formula_a = make_formula("a", "1.0", &["b"], sha_a);
        let formula_b = make_formula("b", "2.0", &[], sha_b);

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula_a, formula_b],
            vec![],
            counter.clone(),
            layout.clone(),
        );

        run(&orchestrator, &["a".to_owned()], true, false).await?;

        // dry_run: nothing installed
        assert!(!layout.cellar().join("a").exists());
        assert!(!layout.cellar().join("b").exists());
        Ok(())
    }
}
