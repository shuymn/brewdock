use std::{
    ffi::OsString,
    fmt::Write as _,
    io,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use crate::{
    error::CellarError,
    materialize::copy_dir_recursive,
    relocate::{
        RelocationScope, build_relocation_manifest, relocate_keg, relocate_keg_with_manifest,
    },
};

const SPIKE_ROOT_ENV: &str = "BREWDOCK_COPY_STRATEGY_SPIKE_ROOT";
const SPIKE_RUNS_ENV: &str = "BREWDOCK_COPY_STRATEGY_SPIKE_RUNS";
const SPIKE_OUTPUT_ENV: &str = "BREWDOCK_COPY_STRATEGY_SPIKE_OUTPUT";

#[derive(Debug, Clone)]
struct Sample {
    name: String,
    source: PathBuf,
    total_files: usize,
    placeholder_files: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Strategy {
    Current,
    ManifestTargeted,
    ClonefileFirst,
    ClonefileAndManifest,
}

impl Strategy {
    const fn label(self) -> &'static str {
        match self {
            Self::Current => "current recursive copy + full relocation walk",
            Self::ManifestTargeted => "recursive copy + manifest-targeted relocation",
            Self::ClonefileFirst => "clonefile-first copy + full relocation walk",
            Self::ClonefileAndManifest => "clonefile-first copy + manifest-targeted relocation",
        }
    }

    const fn short_name(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::ManifestTargeted => "manifest-targeted",
            Self::ClonefileFirst => "clonefile-first",
            Self::ClonefileAndManifest => "clonefile+manifest",
        }
    }

    const fn risk_rank(self) -> u8 {
        match self {
            Self::Current => 3,
            Self::ManifestTargeted => 0,
            Self::ClonefileFirst => 1,
            Self::ClonefileAndManifest => 2,
        }
    }
}

#[derive(Debug, Clone)]
struct StrategySummary {
    strategy: Strategy,
    durations: Vec<Duration>,
}

impl StrategySummary {
    fn median(&self) -> Duration {
        median_duration(&self.durations)
    }
}

fn spike_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    std::env::var_os(SPIKE_ROOT_ENV)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {SPIKE_ROOT_ENV}").into())
}

fn spike_runs() -> usize {
    std::env::var(SPIKE_RUNS_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|runs| *runs > 0)
        .unwrap_or(3)
}

fn collect_samples(root: &Path) -> Result<Vec<Sample>, Box<dyn std::error::Error>> {
    let mut samples = std::fs::read_dir(root)?
        .map(|entry| -> Result<Sample, Box<dyn std::error::Error>> {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                return Err(
                    format!("non-directory sample entry: {}", entry.path().display()).into(),
                );
            }

            let source = entry.path();
            let all_files = crate::fs::walk_files(&source)?;
            let manifest = build_relocation_manifest(&source)?;
            Ok(Sample {
                name: entry.file_name().to_string_lossy().into_owned(),
                source,
                total_files: all_files.len(),
                placeholder_files: manifest.len(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    samples.sort_by(|left, right| left.name.cmp(&right.name));
    if samples.is_empty() {
        return Err(format!("no samples found under {}", root.display()).into());
    }
    Ok(samples)
}

fn run_strategy(
    sample: &Sample,
    strategy: Strategy,
    runs: usize,
) -> Result<StrategySummary, Box<dyn std::error::Error>> {
    let durations = (0..runs)
        .map(|run| run_once(sample, strategy, run))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(StrategySummary {
        strategy,
        durations,
    })
}

fn run_once(
    sample: &Sample,
    strategy: Strategy,
    run: usize,
) -> Result<Duration, Box<dyn std::error::Error>> {
    let tempdir = tempfile::tempdir()?;
    let prefix = tempdir.path().join("prefix");
    let keg_path = prefix
        .join("Cellar")
        .join(&sample.name)
        .join(format!("spike-{run}"));
    let start = Instant::now();

    match strategy {
        Strategy::Current => {
            copy_dir_recursive(&sample.source, &keg_path)?;
            relocate_keg(&keg_path, &prefix, RelocationScope::Full)?;
        }
        Strategy::ManifestTargeted => {
            let manifest = build_relocation_manifest(&sample.source)?;
            copy_dir_recursive(&sample.source, &keg_path)?;
            relocate_keg_with_manifest(&keg_path, &prefix, RelocationScope::Full, &manifest)?;
        }
        Strategy::ClonefileFirst => {
            clone_dir(&sample.source, &keg_path)?;
            relocate_keg(&keg_path, &prefix, RelocationScope::Full)?;
        }
        Strategy::ClonefileAndManifest => {
            let manifest = build_relocation_manifest(&sample.source)?;
            clone_dir(&sample.source, &keg_path)?;
            relocate_keg_with_manifest(&keg_path, &prefix, RelocationScope::Full, &manifest)?;
        }
    }

    Ok(start.elapsed())
}

fn clone_dir(source: &Path, destination: &Path) -> Result<(), CellarError> {
    std::fs::create_dir_all(destination)?;
    let mut source_arg = OsString::from(source.as_os_str());
    source_arg.push("/.");
    let status = Command::new("cp")
        .arg("-cR")
        .arg(&source_arg)
        .arg(destination)
        .status()
        .map_err(|error| {
            CellarError::Io(io::Error::other(format!("cp -cR failed to start: {error}")))
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(cp_exit_failure(status).into())
    }
}

fn cp_exit_failure(status: std::process::ExitStatus) -> io::Error {
    io::Error::other(format!("cp -cR exited with status {status}"))
}

fn median_duration(values: &[Duration]) -> Duration {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        let left = sorted[mid - 1].as_secs_f64();
        let right = sorted[mid].as_secs_f64();
        Duration::from_secs_f64(f64::midpoint(left, right))
    }
}

fn format_duration(duration: Duration) -> String {
    format!("{:.1}ms", duration.as_secs_f64() * 1_000.0)
}

fn choose_next_strategy(summaries: &[StrategySummary]) -> Strategy {
    let baseline = summaries
        .iter()
        .find(|summary| summary.strategy == Strategy::Current)
        .map(StrategySummary::median)
        .unwrap_or_default();
    let mut candidates = summaries
        .iter()
        .filter(|summary| summary.strategy != Strategy::Current)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|summary| summary.median());
    let Some(fastest) = candidates.first().copied() else {
        return Strategy::Current;
    };

    let lower_risk = candidates
        .iter()
        .copied()
        .filter(|summary| summary.strategy.risk_rank() < fastest.strategy.risk_rank())
        .find(|summary| {
            let fastest_secs = fastest.median().as_secs_f64();
            let candidate_secs = summary.median().as_secs_f64();
            (candidate_secs - fastest_secs) / baseline.as_secs_f64().max(0.001) <= 0.10
        });

    lower_risk.map_or(fastest.strategy, |summary| summary.strategy)
}

fn render_markdown(sample: &Sample, summaries: &[StrategySummary], chosen: Strategy) -> String {
    let baseline = summaries
        .iter()
        .find(|summary| summary.strategy == Strategy::Current)
        .map(StrategySummary::median)
        .unwrap_or_default();
    let mut markdown = String::new();
    markdown.push_str("## Copy Strategy Spike\n\n");
    markdown.push_str(
        "_Representative extracted bottles copied from the VM store after `bd install jq wget`; medians are aggregated on the host across the configured spike runs._\n\n",
    );
    let _ = writeln!(
        markdown,
        "### {}\n\nFiles: {}. Placeholder-bearing files: {}.\n",
        sample.name, sample.total_files, sample.placeholder_files
    );
    markdown.push_str("| Strategy | Median | Delta vs current |\n");
    markdown.push_str("|---|---:|---:|\n");
    for summary in summaries {
        let delta = summary.median().as_secs_f64() - baseline.as_secs_f64();
        let _ = writeln!(
            markdown,
            "| {} | {} | {:+.1}ms |",
            summary.strategy.label(),
            format_duration(summary.median()),
            delta * 1_000.0
        );
    }
    markdown.push('\n');
    let _ = writeln!(
        markdown,
        "Chosen next implementation choice: `{}`.\n",
        chosen.short_name()
    );
    markdown.push_str(
        "Rejected from next-step consideration: hardlink-first copy would let keg writes alias shared store state, which breaks the fail-closed rollback boundary even if it benchmarks well.\n",
    );
    markdown
}

fn write_optional_output(markdown: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(path) = std::env::var_os(SPIKE_OUTPUT_ENV) {
        std::fs::write(path, markdown)?;
    }
    Ok(())
}

#[test]
#[ignore = "benchmark-only spike harness, replayed by tests/vm-pipeline-baseline.sh"]
fn copy_strategy_spike_benchmark() -> Result<(), Box<dyn std::error::Error>> {
    let root = spike_root()?;
    let runs = spike_runs();
    let samples = collect_samples(&root)?;

    let mut sections = Vec::new();
    for sample in samples {
        let summaries = [
            Strategy::Current,
            Strategy::ManifestTargeted,
            Strategy::ClonefileFirst,
            Strategy::ClonefileAndManifest,
        ]
        .into_iter()
        .map(|strategy| run_strategy(&sample, strategy, runs))
        .collect::<Result<Vec<_>, _>>()?;
        let chosen = choose_next_strategy(&summaries);
        sections.push(render_markdown(&sample, &summaries, chosen));
    }

    let markdown = sections.join("\n");
    println!("{markdown}");
    write_optional_output(&markdown)?;
    Ok(())
}
