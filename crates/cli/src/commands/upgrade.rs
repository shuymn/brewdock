use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::{Verbosity, output};

/// Runs the upgrade command.
///
/// # Errors
///
/// Returns an error if the upgrade orchestration fails.
pub async fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    formulae: &[String],
    dry_run: bool,
    verbosity: Verbosity,
) -> Result<()> {
    let formula_names: Vec<&str> = formulae.iter().map(String::as_str).collect();

    if dry_run {
        let plan = orchestrator
            .plan_upgrade(&formula_names)
            .await
            .context("upgrade planning failed")?;
        if !verbosity.is_quiet() {
            print!("{}", output::render_upgrade_plan(&plan));
        }
        return Ok(());
    }

    let upgraded = orchestrator
        .upgrade(&formula_names)
        .await
        .context("upgrade failed")?;
    if !verbosity.is_quiet() {
        print!("{}", output::render_upgrade_summary(&upgraded));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use brewdock_core::{InstallMethod, SourceBuildPlan, UpgradePlanEntry};

    use crate::output;

    const SHA_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    #[test]
    fn test_render_upgrade_plan_includes_method() {
        let plan = vec![UpgradePlanEntry {
            name: "a".to_owned(),
            from_version: "1.0".to_owned(),
            to_version: "2.0".to_owned(),
            method: InstallMethod::Source(SourceBuildPlan {
                formula_name: "a".to_owned(),
                version: "2.0".to_owned(),
                source_url: "https://example.com/a-2.0.tar.gz".to_owned(),
                source_checksum: Some(SHA_C.to_owned()),
                build_dependencies: Vec::new(),
                runtime_dependencies: Vec::new(),
                prefix: std::path::PathBuf::from("/opt/homebrew"),
                cellar_path: std::path::PathBuf::from("/opt/homebrew/Cellar/a/2.0"),
            }),
        }];

        let rendered = output::render_upgrade_plan(&plan);
        assert!(rendered.contains("a 1.0 -> 2.0 [source]"));
    }
}
