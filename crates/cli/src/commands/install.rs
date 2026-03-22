use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::{Verbosity, output};

/// Runs the install command.
///
/// # Errors
///
/// Returns an error if the install orchestration fails.
pub async fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    formulae: &[String],
    dry_run: bool,
    verbosity: Verbosity,
) -> Result<()> {
    let formula_names: Vec<&str> = formulae.iter().map(String::as_str).collect();

    if dry_run {
        let plan = orchestrator
            .plan_install(&formula_names)
            .await
            .context("install planning failed")?;
        if !verbosity.is_quiet() {
            print!("{}", output::render_install_plan(&plan));
        }
        return Ok(());
    }

    let installed = orchestrator
        .install(&formula_names)
        .await
        .context("install failed")?;
    if !verbosity.is_quiet() {
        print!("{}", output::render_install_summary(&installed));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use brewdock_core::{InstallMethod, PlanEntry, SourceBuildPlan};

    use crate::output;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

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

        let rendered = output::render_install_plan(&plan);
        assert!(rendered.contains("a 1.0 [bottle:arm64_sonoma]"));
        assert!(rendered.contains("b 2.0 [source]"));
    }
}
