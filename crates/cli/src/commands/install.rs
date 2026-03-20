use std::fmt::Write;

use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator, PlanEntry};

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
    let formula_names: Vec<&str> = formulae.iter().map(String::as_str).collect();

    if dry_run {
        let plan = orchestrator
            .plan_install(&formula_names)
            .await
            .context("install planning failed")?;
        if !quiet {
            print!("{}", render_install_plan(&plan));
        }
        return Ok(());
    }

    let installed = orchestrator
        .install(&formula_names)
        .await
        .context("install failed")?;
    if !quiet {
        for name in &installed {
            println!("Installed {name}");
        }
    }
    Ok(())
}

fn render_install_plan(plan: &[PlanEntry]) -> String {
    if plan.is_empty() {
        return "Nothing to install\n".to_owned();
    }

    let mut output = String::from("Would install:\n");
    for entry in plan {
        let _ = writeln!(
            output,
            "  {} {} [{}]",
            entry.name, entry.version, entry.method
        );
    }
    output
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicUsize};

    use brewdock_core::{InstallMethod, Layout, PlanEntry, SourceBuildPlan};

    use super::*;
    use crate::testutil::{SHA_A, SHA_B, create_bottle_tar_gz, make_formula, make_orchestrator};

    #[tokio::test]
    async fn test_commands_install_single_formula() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let formula = make_formula("a", "1.0", &[], SHA_A);
        let tar = create_bottle_tar_gz("a", "1.0", &[("bin/a_tool", b"#!/bin/sh\necho a")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator =
            make_orchestrator(vec![formula], vec![(SHA_A, tar)], counter, layout.clone())?;

        run(&orchestrator, &["a".to_owned()], false, false).await?;

        assert!(layout.cellar().join("a/1.0/bin/a_tool").exists());
        assert!(layout.prefix().join("bin/a_tool").is_symlink());
        Ok(())
    }

    #[tokio::test]
    async fn test_commands_install_with_deps() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let formula_a = make_formula("a", "1.0", &["b"], SHA_A);
        let formula_b = make_formula("b", "2.0", &[], SHA_B);

        let tar_a = create_bottle_tar_gz("a", "1.0", &[("bin/a_tool", b"#!/bin/sh")])?;
        let tar_b = create_bottle_tar_gz("b", "2.0", &[("bin/b_tool", b"#!/bin/sh")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula_a, formula_b],
            vec![(SHA_A, tar_a), (SHA_B, tar_b)],
            counter,
            layout.clone(),
        )?;

        run(&orchestrator, &["a".to_owned()], false, false).await?;

        assert!(layout.cellar().join("a/1.0/bin/a_tool").exists());
        assert!(layout.cellar().join("b/2.0/bin/b_tool").exists());
        Ok(())
    }

    #[tokio::test]
    async fn test_dry_run_install_does_not_execute() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let formula = make_formula("a", "1.0", &[], SHA_A);
        let tar = create_bottle_tar_gz("a", "1.0", &[("bin/a_tool", b"#!/bin/sh")])?;

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula],
            vec![(SHA_A, tar)],
            counter.clone(),
            layout.clone(),
        )?;

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

        let formula_a = make_formula("a", "1.0", &["b"], SHA_A);
        let formula_b = make_formula("b", "2.0", &[], SHA_B);

        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(
            vec![formula_a, formula_b],
            vec![],
            counter.clone(),
            layout.clone(),
        )?;

        run(&orchestrator, &["a".to_owned()], true, false).await?;

        // dry_run: nothing installed
        assert!(!layout.cellar().join("a").exists());
        assert!(!layout.cellar().join("b").exists());
        Ok(())
    }

    #[test]
    fn test_render_install_plan_includes_method() {
        let plan = vec![
            PlanEntry {
                name: "a".to_owned(),
                version: "1.0".to_owned(),
                method: InstallMethod::Bottle(brewdock_formula::SelectedBottle {
                    tag: "arm64_sonoma".to_owned(),
                    url: "https://example.com/a.tar.gz".to_owned(),
                    sha256: SHA_A.to_owned(),
                    cellar: brewdock_formula::CellarType::Any,
                }),
            },
            PlanEntry {
                name: "b".to_owned(),
                version: "2.0".to_owned(),
                method: InstallMethod::Source(SourceBuildPlan {
                    formula_name: "b".to_owned(),
                    version: "2.0".to_owned(),
                    source_url: "https://example.com/b-2.0.tar.gz".to_owned(),
                    source_checksum: Some(SHA_B.to_owned()),
                    build_dependencies: Vec::new(),
                    runtime_dependencies: Vec::new(),
                    prefix: std::path::PathBuf::from("/opt/homebrew"),
                    cellar_path: std::path::PathBuf::from("/opt/homebrew/Cellar/b/2.0"),
                }),
            },
        ];

        let rendered = render_install_plan(&plan);
        assert!(rendered.contains("a 1.0 [bottle:arm64_sonoma]"));
        assert!(rendered.contains("b 2.0 [source]"));
    }
}
