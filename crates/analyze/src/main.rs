#![warn(clippy::pedantic, clippy::nursery)]

//! Formula compatibility analysis tool for brewdock.
//!
//! Analyzes Homebrew formulae to determine whether they can be installed
//! by brewdock, checking both supportability (bottle availability, disabled
//! status, pour restrictions) and `post_install` compatibility. Optional
//! `test do` analysis reports both parse coverage and v1 runtime-lowering coverage.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use brewdock_analysis::{analyze_post_install_all, analyze_test_do_all};
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

    /// Analyze `test do` blocks and report runtime subset support.
    #[arg(long)]
    test_do: bool,

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

#[allow(clippy::struct_excessive_bools)]
#[derive(serde::Serialize)]
struct FormulaReport {
    name: String,
    verdict: Verdict,
    has_bottle: bool,
    // post_install (always analyzed)
    has_post_install: bool,
    post_install_parse_ok: bool,
    post_install_runtime_ok: bool,
    post_install_features: Vec<String>,
    supportability_error: Option<String>,
    post_install_error: Option<String>,
    // test_do (opt-in via --test-do)
    #[serde(skip_serializing_if = "Option::is_none")]
    has_test_do: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_do_parse_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_do_runtime_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_do_features: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    test_do_error: Option<String>,
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

/// `(has_block, parse_ok, runtime_ok, error, feature_names)`.
type AnalysisCheck = (bool, bool, bool, Option<String>, Vec<String>);

fn summarize_block_analysis(
    feature_names: Vec<&'static str>,
    lowering_err: Option<brewdock_analysis::AnalysisError>,
) -> AnalysisCheck {
    let runtime_ok = lowering_err.is_none();
    let error = lowering_err.map(|e| e.to_string());
    let features = feature_names.into_iter().map(str::to_owned).collect();
    (true, true, runtime_ok, error, features)
}

fn check_post_install_source(source: &str, version: &str) -> AnalysisCheck {
    match analyze_post_install_all(source, version) {
        Ok(Some(a)) => summarize_block_analysis(a.features.names(), a.lowering.err()),
        Ok(None) => (false, true, true, None, Vec::new()),
        Err(e) => (true, false, false, Some(e.to_string()), Vec::new()),
    }
}

fn check_test_do_source(source: &str) -> AnalysisCheck {
    match analyze_test_do_all(source) {
        Ok(Some(a)) => summarize_block_analysis(a.features.names(), a.lowering.err()),
        Ok(None) => (false, true, true, None, Vec::new()),
        Err(e) => (true, false, false, Some(e.to_string()), Vec::new()),
    }
}

async fn analyze_formula_api(
    repo: &HttpFormulaRepository,
    name: &str,
    check_test_do: bool,
) -> Result<FormulaReport> {
    let formula: Formula = repo.formula(name).await?;

    let supportability_error = check_supportability(&formula, HOST_TAG).err();
    let has_bottle = formula.versions.bottle && select_bottle(&formula, HOST_TAG).is_some();

    let ruby_source = if formula.post_install_defined || check_test_do {
        if let Some(ruby_source_path) = formula.ruby_source_path.as_deref() {
            Some(repo.ruby_source(ruby_source_path).await)
        } else {
            None
        }
    } else {
        None
    };

    let (
        has_post_install,
        post_install_parse_ok,
        post_install_runtime_ok,
        post_install_error,
        post_install_features,
    ) = if formula.post_install_defined {
        match ruby_source.as_ref() {
            Some(Ok(source)) => check_post_install_source(source, &formula.versions.stable),
            Some(Err(error)) => (
                true,
                false,
                false,
                Some(format!("fetch failed: {error}")),
                Vec::new(),
            ),
            None => (
                true,
                false,
                false,
                Some("no ruby_source_path".to_owned()),
                Vec::new(),
            ),
        }
    } else {
        (false, true, true, None, Vec::new())
    };

    let (has_test_do, test_do_parse_ok, test_do_runtime_ok, test_do_error, test_do_features) =
        if check_test_do {
            match ruby_source.as_ref() {
                Some(Ok(source)) => {
                    let (has, parse_ok, runtime_ok, error, features) = check_test_do_source(source);
                    (
                        Some(has),
                        Some(parse_ok),
                        Some(runtime_ok),
                        error,
                        Some(features),
                    )
                }
                Some(Err(error)) => (
                    Some(true),
                    Some(false),
                    Some(false),
                    Some(format!("fetch failed: {error}")),
                    Some(Vec::new()),
                ),
                None => (
                    Some(true),
                    Some(false),
                    Some(false),
                    Some("no ruby_source_path".to_owned()),
                    Some(Vec::new()),
                ),
            }
        } else {
            (None, None, None, None, None)
        };

    let verdict = if supportability_error.is_some() {
        Verdict::Unsupported
    } else if !post_install_runtime_ok && has_post_install {
        Verdict::PostInstallUnsupported
    } else {
        Verdict::Ok
    };

    Ok(FormulaReport {
        name: name.to_owned(),
        verdict,
        has_bottle,
        has_post_install,
        post_install_parse_ok,
        post_install_runtime_ok,
        post_install_features,
        has_test_do,
        test_do_parse_ok,
        test_do_runtime_ok,
        test_do_features,
        supportability_error: supportability_error.map(|e| e.to_string()),
        post_install_error,
        test_do_error,
    })
}

fn analyze_formula_local(
    name: &str,
    tap_path: &Path,
    check_test_do: bool,
) -> Result<FormulaReport> {
    let rb_path =
        formula_rb_path(tap_path, name).with_context(|| format!("empty formula name: {name}"))?;

    let source = std::fs::read_to_string(&rb_path)
        .with_context(|| format!("formula {name}.rb not found at {}", rb_path.display()))?;

    let (
        has_post_install,
        post_install_parse_ok,
        post_install_runtime_ok,
        post_install_error,
        post_install_features,
    ) = check_post_install_source(&source, "0.0.0");
    let (has_test_do, test_do_parse_ok, test_do_runtime_ok, test_do_error, test_do_features) =
        if check_test_do {
            let (has, parse_ok, runtime_ok, error, features) = check_test_do_source(&source);
            (
                Some(has),
                Some(parse_ok),
                Some(runtime_ok),
                error,
                Some(features),
            )
        } else {
            (None, None, None, None, None)
        };

    // Local analysis cannot check supportability (no JSON metadata).
    let verdict = if !post_install_runtime_ok && has_post_install {
        Verdict::PostInstallUnsupported
    } else {
        Verdict::Ok
    };

    Ok(FormulaReport {
        name: name.to_owned(),
        verdict,
        has_bottle: false, // unknown from local .rb
        has_post_install,
        post_install_parse_ok,
        post_install_runtime_ok,
        post_install_features,
        has_test_do,
        test_do_parse_ok,
        test_do_runtime_ok,
        test_do_features,
        supportability_error: None,
        post_install_error,
        test_do_error,
    })
}

fn error_report(name: String, error: impl std::fmt::Display) -> FormulaReport {
    FormulaReport {
        name,
        verdict: Verdict::Error,
        has_bottle: false,
        has_post_install: false,
        post_install_parse_ok: false,
        post_install_runtime_ok: false,
        post_install_features: Vec::new(),
        has_test_do: None,
        test_do_parse_ok: None,
        test_do_runtime_ok: None,
        test_do_features: None,
        supportability_error: Some(format!("{error}")),
        post_install_error: None,
        test_do_error: None,
    }
}

fn print_table(reports: &[FormulaReport], include_test_do: bool) {
    // Column widths: 30 + 12 + 8 + 10 + 10 [+ 10 + 10] + "error"
    if include_test_do {
        println!(
            "{:<30} {:<12} {:<8} {:<10} {:<10} {:<10} {:<10} error",
            "Formula", "verdict", "bottle", "pi_parse", "pi_rt", "td_parse", "td_rt"
        );
    } else {
        println!(
            "{:<30} {:<12} {:<8} {:<10} {:<10} error",
            "Formula", "verdict", "bottle", "pi_parse", "pi_rt"
        );
    }
    let sep_width = if include_test_do { 110 } else { 90 };
    println!("{}", "-".repeat(sep_width));

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
        let pi_parse = match (report.has_post_install, report.post_install_parse_ok) {
            (false, _) => "-",
            (true, true) => "ok",
            (true, false) => "FAIL",
        };
        let pi_rt = match (report.has_post_install, report.post_install_runtime_ok) {
            (false, _) => "-",
            (true, true) => "ok",
            (true, false) => "FAIL",
        };
        let error = report
            .supportability_error
            .as_deref()
            .or(report.post_install_error.as_deref())
            .or(report.test_do_error.as_deref())
            .unwrap_or("");
        if include_test_do {
            let td_parse = match (report.has_test_do, report.test_do_parse_ok) {
                (Some(true), Some(true)) => "ok",
                (Some(true), Some(false)) => "FAIL",
                _ => "-",
            };
            let td_rt = match (report.has_test_do, report.test_do_runtime_ok) {
                (Some(true), Some(true)) => "ok",
                (Some(true), Some(false)) => "FAIL",
                _ => "-",
            };
            println!(
                "{:<30} {:<12} {:<8} {:<10} {:<10} {:<10} {:<10} {}",
                report.name, verdict_str, bottle, pi_parse, pi_rt, td_parse, td_rt, error
            );
        } else {
            println!(
                "{:<30} {:<12} {:<8} {:<10} {:<10} {}",
                report.name, verdict_str, bottle, pi_parse, pi_rt, error
            );
        }
    }

    println!("{}", "-".repeat(sep_width));
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
            match analyze_formula_local(name, &cli.tap_path, cli.test_do) {
                Ok(report) => reports.push(report),
                Err(e) => reports.push(error_report(name.clone(), e)),
            }
        }
    } else {
        let repo = HttpFormulaRepository::new();
        for name in &formulas {
            match analyze_formula_api(&repo, name, cli.test_do).await {
                Ok(report) => reports.push(report),
                Err(e) => reports.push(error_report(name.clone(), e)),
            }
        }
    }

    match cli.format {
        OutputFormat::Table => print_table(&reports, cli.test_do),
        OutputFormat::Json => print_json(&reports)?,
    }

    Ok(())
}
