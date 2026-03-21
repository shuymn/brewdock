use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaRepository, Orchestrator};

use crate::{Verbosity, output};

/// Runs the info command.
///
/// # Errors
///
/// Returns an error if the info lookup fails.
pub async fn run<R: FormulaRepository, D: BottleDownloader>(
    orchestrator: &Orchestrator<R, D>,
    formula: &str,
    verbosity: Verbosity,
) -> Result<()> {
    let info = orchestrator
        .info(formula)
        .await
        .context("info lookup failed")?;

    if !verbosity.is_quiet() {
        print!("{}", output::render_info(&info));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::output;

    #[test]
    fn test_render_info_format() {
        let info = brewdock_core::FormulaInfo {
            name: "jq".to_owned(),
            version: "1.7".to_owned(),
            desc: Some("Lightweight JSON processor".to_owned()),
            homepage: Some("https://jqlang.github.io/jq/".to_owned()),
            license: Some("MIT".to_owned()),
            keg_only: false,
            dependencies: vec!["oniguruma".to_owned()],
            bottle_available: true,
            installed_version: Some("1.7".to_owned()),
        };

        let rendered = output::render_info(&info);
        assert!(rendered.contains("jq 1.7"));
        assert!(rendered.contains("Description: Lightweight JSON processor"));
        assert!(rendered.contains("Bottle: available"));
        assert!(rendered.contains("Installed: 1.7"));
        assert!(rendered.contains("Dependencies: oniguruma"));
    }

    #[test]
    fn test_render_info_not_installed() {
        let info = brewdock_core::FormulaInfo {
            name: "wget".to_owned(),
            version: "1.24".to_owned(),
            desc: None,
            homepage: None,
            license: None,
            keg_only: true,
            dependencies: vec![],
            bottle_available: false,
            installed_version: None,
        };

        let rendered = output::render_info(&info);
        assert!(rendered.contains("[keg-only]"));
        assert!(rendered.contains("Installed: no"));
        assert!(rendered.contains("Bottle: not available"));
    }
}
