use std::{fs::OpenOptions, path::PathBuf};

use tracing_subscriber::{
    EnvFilter,
    fmt::{self, format::FmtSpan},
};

use crate::Verbosity;

const BENCHMARK_FILE_ENV: &str = "BREWDOCK_BENCHMARK_FILE";

/// Initializes tracing output for normal CLI runs and benchmark captures.
///
/// # Errors
///
/// Returns an error when benchmark tracing is enabled and the configured output
/// file cannot be opened for append.
pub fn init_tracing(verbosity: Verbosity) -> Result<(), std::io::Error> {
    let subscriber = subscriber(verbosity)?;

    // Intentionally discard: fails harmlessly if a subscriber is already set (e.g., in tests).
    let _ = tracing::subscriber::set_global_default(subscriber);
    Ok(())
}

fn subscriber(
    verbosity: Verbosity,
) -> Result<Box<dyn tracing::Subscriber + Send + Sync>, std::io::Error> {
    let filter = env_filter(verbosity);

    if let Some(path) = benchmark_log_path() {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(benchmark_subscriber(filter, file))
    } else {
        Ok(default_subscriber(filter))
    }
}

fn env_filter(verbosity: Verbosity) -> EnvFilter {
    let level = match verbosity {
        Verbosity::Verbose => "debug",
        Verbosity::Normal => "info",
        Verbosity::Quiet => "error",
    };

    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level))
}

fn benchmark_log_path() -> Option<PathBuf> {
    benchmark_log_path_from_value(std::env::var_os(BENCHMARK_FILE_ENV))
}

fn benchmark_log_path_from_value(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn default_subscriber(filter: EnvFilter) -> Box<dyn tracing::Subscriber + Send + Sync> {
    Box::new(fmt::Subscriber::builder().with_env_filter(filter).finish())
}

fn benchmark_subscriber(
    filter: EnvFilter,
    file: std::fs::File,
) -> Box<dyn tracing::Subscriber + Send + Sync> {
    Box::new(
        fmt::Subscriber::builder()
            .with_env_filter(filter)
            .json()
            .with_span_events(FmtSpan::CLOSE)
            .with_current_span(true)
            .with_span_list(true)
            .with_writer(file)
            .finish(),
    )
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn test_benchmark_log_path_none_when_env_missing() {
        assert!(benchmark_log_path_from_value(None).is_none());
    }

    #[test]
    fn test_benchmark_subscriber_writes_close_event_json() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempdir()?;
        let path = dir.path().join("benchmark.jsonl");
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let subscriber = benchmark_subscriber(EnvFilter::new("info"), file);

        tracing::dispatcher::with_default(&tracing::Dispatch::new(subscriber), || {
            let span = tracing::info_span!(
                "bd.phase",
                operation = "update",
                phase = "fetch-formula-index",
                target = "formula-index"
            );
            let _entered = span.enter();
        });

        let mut output = String::new();
        OpenOptions::new()
            .read(true)
            .open(&path)?
            .read_to_string(&mut output)?;
        assert!(output.contains("\"message\":\"close\""));
        assert!(output.contains("fetch-formula-index"));
        assert!(output.contains("\"time.busy\""));
        Ok(())
    }
}
