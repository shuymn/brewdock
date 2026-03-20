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
) -> Result<()> {
    let installed = orchestrator
        .install(formulae)
        .await
        .context("install failed")?;
    for name in &installed {
        println!("Installed {name}");
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

        run(&orchestrator, &["a".to_owned()]).await?;

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

        run(&orchestrator, &["a".to_owned()]).await?;

        assert!(layout.cellar().join("a/1.0/bin/a_tool").exists());
        assert!(layout.cellar().join("b/2.0/bin/b_tool").exists());
        Ok(())
    }
}
