#![warn(clippy::pedantic, clippy::nursery)]

//! Formula compatibility analysis tool for brewdock.
//!
//! Analyzes Homebrew formulae to determine whether they can be installed
//! by brewdock, checking both supportability (bottle availability, disabled
//! status, pour restrictions) and `post_install` compatibility.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use brewdock_analysis::{extract_post_install_block, validate_post_install};
use brewdock_formula::{
    Formula, FormulaRepository, HttpFormulaRepository, check_supportability, select_bottle,
};
use clap::Parser;

/// Analyze Homebrew formula compatibility with brewdock.
#[derive(Parser)]
#[command(
    name = "bd-analyze",
    about = "Formula compatibility analysis for brewdock"
)]
struct Cli {
    /// Formula names to analyze. Pass `-` to read from stdin (one per line).
    formulas: Vec<String>,

    /// Use local homebrew-core `.rb` files instead of the JSON API.
    /// Only checks `post_install` compatibility (no bottle/supportability info).
    #[arg(long)]
    local: bool,

    /// Path to local homebrew-core tap (used with `--local`).
    #[arg(
        long,
        default_value = "/opt/homebrew/Library/Taps/homebrew/homebrew-core"
    )]
    tap_path: PathBuf,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    format: OutputFormat,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum OutputFormat {
    /// Human-readable table.
    Table,
    /// JSON output.
    Json,
}

const HOST_TAG: &str = "arm64_sequoia";

/// Overall installability verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Verdict {
    /// Fully installable by brewdock.
    Ok,
    /// Installable but `post_install` will be skipped.
    PostInstallUnsupported,
    /// Cannot be installed by brewdock.
    Unsupported,
    /// Analysis failed (e.g. network error, missing file).
    Error,
}

#[derive(serde::Serialize)]
struct FormulaReport {
    name: String,
    verdict: Verdict,
    has_bottle: bool,
    has_post_install: bool,
    post_install_ok: bool,
    supportability_error: Option<String>,
    post_install_error: Option<String>,
}

fn formula_rb_path(tap_path: &Path, name: &str) -> Option<PathBuf> {
    let first_char = name.chars().next()?;
    Some(
        tap_path
            .join("Formula")
            .join(first_char.to_string())
            .join(format!("{name}.rb")),
    )
}

fn check_post_install_source(source: &str, version: &str) -> (bool, bool, Option<String>) {
    let has_post_install = extract_post_install_block(source).is_ok();
    if !has_post_install {
        return (false, true, None);
    }
    match validate_post_install(source, version) {
        Ok(()) => (true, true, None),
        Err(e) => (true, false, Some(e.to_string())),
    }
}

async fn analyze_formula_api(repo: &HttpFormulaRepository, name: &str) -> Result<FormulaReport> {
    let formula: Formula = repo.formula(name).await?;

    let supportability_error = check_supportability(&formula, HOST_TAG).err();
    let has_bottle = formula.versions.bottle && select_bottle(&formula, HOST_TAG).is_some();

    let (has_post_install, post_install_ok, post_install_error) = if formula.post_install_defined {
        if let Some(ruby_source_path) = formula.ruby_source_path.as_deref() {
            match repo.ruby_source(ruby_source_path).await {
                Ok(source) => check_post_install_source(&source, &formula.versions.stable),
                Err(e) => (true, false, Some(format!("fetch failed: {e}"))),
            }
        } else {
            (true, false, Some("no ruby_source_path".to_owned()))
        }
    } else {
        (false, true, None)
    };

    let verdict = if supportability_error.is_some() {
        Verdict::Unsupported
    } else if !post_install_ok && has_post_install {
        Verdict::PostInstallUnsupported
    } else {
        Verdict::Ok
    };

    Ok(FormulaReport {
        name: name.to_owned(),
        verdict,
        has_bottle,
        has_post_install,
        post_install_ok,
        supportability_error: supportability_error.map(|e| e.to_string()),
        post_install_error,
    })
}

fn analyze_formula_local(name: &str, tap_path: &Path) -> Result<FormulaReport> {
    let rb_path =
        formula_rb_path(tap_path, name).with_context(|| format!("empty formula name: {name}"))?;

    let source = std::fs::read_to_string(&rb_path)
        .with_context(|| format!("formula {name}.rb not found at {}", rb_path.display()))?;

    let (has_post_install, post_install_ok, post_install_error) =
        check_post_install_source(&source, "0.0.0");

    // Local analysis cannot check supportability (no JSON metadata).
    let verdict = if !post_install_ok && has_post_install {
        Verdict::PostInstallUnsupported
    } else {
        Verdict::Ok
    };

    Ok(FormulaReport {
        name: name.to_owned(),
        verdict,
        has_bottle: false, // unknown from local .rb
        has_post_install,
        post_install_ok,
        supportability_error: None,
        post_install_error,
    })
}

fn error_report(name: String, error: impl std::fmt::Display) -> FormulaReport {
    FormulaReport {
        name,
        verdict: Verdict::Error,
        has_bottle: false,
        has_post_install: false,
        post_install_ok: false,
        supportability_error: Some(format!("{error}")),
        post_install_error: None,
    }
}

fn print_table(reports: &[FormulaReport]) {
    println!(
        "{:<30} {:<12} {:<8} {:<12} error",
        "Formula", "verdict", "bottle", "post_install"
    );
    println!("{}", "-".repeat(100));

    let mut counts = [0u32; 4]; // ok, post_install_unsupported, unsupported, error

    for report in reports {
        let verdict_str = match report.verdict {
            Verdict::Ok => "ok",
            Verdict::PostInstallUnsupported => "post_skip",
            Verdict::Unsupported => "UNSUPPORTED",
            Verdict::Error => "ERROR",
        };
        counts[report.verdict as usize] += 1;

        let bottle = if report.has_bottle { "yes" } else { "-" };
        let post_install = match (report.has_post_install, report.post_install_ok) {
            (false, _) => "-",
            (true, true) => "ok",
            (true, false) => "FAIL",
        };
        let error = report
            .supportability_error
            .as_deref()
            .or(report.post_install_error.as_deref())
            .unwrap_or("");
        println!(
            "{:<30} {:<12} {:<8} {:<12} {}",
            report.name, verdict_str, bottle, post_install, error
        );
    }

    println!("{}", "-".repeat(100));
    println!(
        "Total: {} | ok: {} | post_install skipped: {} | unsupported: {} | error: {}",
        reports.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3],
    );
}

fn print_json(reports: &[FormulaReport]) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(reports)?);
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let formulas: Vec<String> = if cli.formulas.len() == 1 && cli.formulas[0] == "-" {
        std::io::BufRead::lines(std::io::stdin().lock())
            .map(|line| line.map(|l| l.trim().to_owned()))
            .collect::<std::io::Result<Vec<_>>>()?
            .into_iter()
            .filter(|l| !l.is_empty())
            .collect()
    } else {
        cli.formulas
    };

    if formulas.is_empty() {
        bail!("no formula names provided. Pass names as arguments or `-` to read from stdin.");
    }

    let mut reports = Vec::with_capacity(formulas.len());

    if cli.local {
        if !cli.tap_path.join("Formula").exists() {
            bail!(
                "homebrew-core tap not found at {}. Run: brew tap --force homebrew/core",
                cli.tap_path.display()
            );
        }
        for name in &formulas {
            match analyze_formula_local(name, &cli.tap_path) {
                Ok(report) => reports.push(report),
                Err(e) => reports.push(error_report(name.clone(), e)),
            }
        }
    } else {
        let repo = HttpFormulaRepository::new();
        for name in &formulas {
            match analyze_formula_api(&repo, name).await {
                Ok(report) => reports.push(report),
                Err(e) => reports.push(error_report(name.clone(), e)),
            }
        }
    }

    match cli.format {
        OutputFormat::Table => print_table(&reports),
        OutputFormat::Json => print_json(&reports)?,
    }

    Ok(())
}
