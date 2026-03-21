use std::fmt::Write;

use anyhow::{Context, Result};
use brewdock_core::{BottleDownloader, FormulaInfo, FormulaRepository, Orchestrator};

use crate::Verbosity;

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
        print!("{}", render_info(&info));
    }

    Ok(())
}

fn render_info(info: &FormulaInfo) -> String {
    let mut output = format!("{} {}", info.name, info.version);
    if info.keg_only {
        output.push_str(" [keg-only]");
    }
    output.push('\n');

    if let Some(desc) = &info.desc {
        let _ = writeln!(output, "{desc}");
    }
    if let Some(homepage) = &info.homepage {
        let _ = writeln!(output, "{homepage}");
    }
    if let Some(license) = &info.license {
        let _ = writeln!(output, "License: {license}");
    }

    if info.bottle_available {
        let _ = writeln!(output, "Bottle: available");
    } else {
        let _ = writeln!(output, "Bottle: not available");
    }

    if let Some(installed) = &info.installed_version {
        let _ = writeln!(output, "Installed: {installed}");
    } else {
        let _ = writeln!(output, "Not installed");
    }

    if !info.dependencies.is_empty() {
        let _ = writeln!(output, "Dependencies: {}", info.dependencies.join(", "));
    }

    output
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::AtomicUsize};

    use brewdock_core::Layout;

    use super::*;
    use crate::testutil::{SHA_A, make_formula, make_orchestrator, setup_installed_keg};

    #[tokio::test]
    async fn test_info_shows_formula_details() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        let formula = make_formula("a", "1.0", &["b"], SHA_A);
        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout)?;

        run(&orchestrator, "a", Verbosity::Normal).await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_info_shows_installed_version() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());

        setup_installed_keg(&layout, "a", "1.0", true)?;

        let formula = make_formula("a", "1.0", &[], SHA_A);
        let counter = Arc::new(AtomicUsize::new(0));
        let orchestrator = make_orchestrator(vec![formula], vec![], counter, layout)?;

        run(&orchestrator, "a", Verbosity::Normal).await?;
        Ok(())
    }

    #[test]
    fn test_render_info_format() {
        let info = FormulaInfo {
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

        let rendered = render_info(&info);
        assert!(rendered.contains("jq 1.7"));
        assert!(rendered.contains("Lightweight JSON processor"));
        assert!(rendered.contains("Bottle: available"));
        assert!(rendered.contains("Installed: 1.7"));
        assert!(rendered.contains("Dependencies: oniguruma"));
    }

    #[test]
    fn test_render_info_not_installed() {
        let info = FormulaInfo {
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

        let rendered = render_info(&info);
        assert!(rendered.contains("[keg-only]"));
        assert!(rendered.contains("Not installed"));
        assert!(rendered.contains("Bottle: not available"));
    }
}
