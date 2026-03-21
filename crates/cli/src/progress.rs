use std::{
    io::{self, IsTerminal},
    sync::{Arc, Mutex},
    time::Duration,
};

use brewdock_core::{NoopProgressSink, OperationProgressSink, ProgressEvent, SharedProgressSink};
use indicatif::{ProgressBar, ProgressStyle};

use crate::Verbosity;

pub fn progress_sink(verbosity: Verbosity) -> SharedProgressSink {
    if verbosity.is_quiet() {
        return Arc::new(NoopProgressSink);
    }

    Arc::new(ProgressRenderer::new(verbosity, RenderMode::detect()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderMode {
    Tty,
    Plain,
}

impl RenderMode {
    fn detect() -> Self {
        if io::stderr().is_terminal() {
            Self::Tty
        } else {
            Self::Plain
        }
    }
}

#[derive(Debug, Default)]
struct RenderSnapshot {
    operation: Option<&'static str>,
    target: Option<String>,
    phase: Option<&'static str>,
    formula: Option<String>,
    completed_formulae: usize,
}

struct ProgressRenderer {
    verbosity: Verbosity,
    mode: RenderMode,
    state: Mutex<RenderSnapshot>,
    spinner: Option<ProgressBar>,
}

impl ProgressRenderer {
    fn new(verbosity: Verbosity, mode: RenderMode) -> Self {
        let spinner = match mode {
            RenderMode::Tty => {
                let spinner = ProgressBar::new_spinner();
                spinner.set_style(
                    ProgressStyle::with_template("{spinner:.cyan} {msg}")
                        .unwrap_or_else(|_| ProgressStyle::default_spinner()),
                );
                spinner.enable_steady_tick(Duration::from_millis(80));
                Some(spinner)
            }
            RenderMode::Plain => None,
        };

        Self {
            verbosity,
            mode,
            state: Mutex::new(RenderSnapshot::default()),
            spinner,
        }
    }

    fn handle_event(&self, event: &ProgressEvent) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        update_snapshot(&mut state, event);

        match self.mode {
            RenderMode::Tty => self.render_tty(&state, event),
            RenderMode::Plain => self.render_plain(event),
        }
    }

    fn render_tty(&self, state: &RenderSnapshot, event: &ProgressEvent) {
        let Some(spinner) = &self.spinner else {
            return;
        };

        spinner.set_message(render_status_line(state));

        match event {
            ProgressEvent::OperationCompleted { .. } => {
                spinner.finish_and_clear();
            }
            ProgressEvent::OperationFailed { error, .. } => {
                spinner.abandon_with_message(format!("failed: {error}"));
            }
            ProgressEvent::Warning {
                operation,
                target,
                message,
            } => spinner.println(format_warning_line(operation, target, message)),
            ProgressEvent::PhaseStarted {
                operation,
                phase,
                target,
            }
            | ProgressEvent::PhaseCompleted {
                operation,
                phase,
                target,
            } if self.verbosity == Verbosity::Verbose => {
                spinner.println(format_phase_line(operation, phase, target));
            }
            ProgressEvent::FormulaCompleted { operation, name }
                if self.verbosity == Verbosity::Verbose =>
            {
                spinner.println(format!(
                    "done: {} {}",
                    format_operation_label(operation),
                    name
                ));
            }
            ProgressEvent::FormulaFailed {
                operation,
                name,
                error,
            } => spinner.println(format!(
                "failed: {} {} ({error})",
                format_operation_label(operation),
                name
            )),
            _ => {}
        }
    }

    fn render_plain(&self, event: &ProgressEvent) {
        match event {
            ProgressEvent::OperationStarted { operation, target } => {
                eprintln!("starting: {} {target}", format_operation_label(operation));
            }
            ProgressEvent::PhaseStarted {
                operation,
                phase,
                target,
            } if self.verbosity == Verbosity::Verbose => {
                eprintln!("{}", format_phase_line(operation, phase, target));
            }
            ProgressEvent::FormulaCompleted { operation, name } => {
                eprintln!("done: {} {}", format_operation_label(operation), name);
            }
            ProgressEvent::Warning {
                operation,
                target,
                message,
            } => eprintln!("{}", format_warning_line(operation, target, message)),
            ProgressEvent::OperationFailed {
                operation,
                target,
                error,
            } => eprintln!(
                "failed: {} {target} ({error})",
                format_operation_label(operation)
            ),
            _ => {}
        }
    }
}

impl OperationProgressSink for ProgressRenderer {
    fn emit(&self, event: ProgressEvent) {
        self.handle_event(&event);
    }
}

fn update_snapshot(state: &mut RenderSnapshot, event: &ProgressEvent) {
    match event {
        ProgressEvent::OperationStarted { operation, target } => {
            state.operation = Some(*operation);
            state.target = Some(target.clone());
            state.phase = None;
            state.formula = None;
            state.completed_formulae = 0;
        }
        ProgressEvent::PhaseStarted { phase, target, .. } => {
            state.phase = Some(*phase);
            state.target = Some(target.clone());
        }
        ProgressEvent::FormulaStarted { name, .. } | ProgressEvent::FormulaFailed { name, .. } => {
            state.formula = Some(name.clone());
        }
        ProgressEvent::FormulaCompleted { name, .. } => {
            state.formula = Some(name.clone());
            state.completed_formulae += 1;
        }
        ProgressEvent::OperationCompleted { .. } | ProgressEvent::OperationFailed { .. } => {
            state.phase = None;
        }
        ProgressEvent::PhaseCompleted { .. }
        | ProgressEvent::PhaseFailed { .. }
        | ProgressEvent::Warning { .. } => {}
    }
}

fn render_status_line(state: &RenderSnapshot) -> String {
    let operation = state.operation.map_or("working", format_operation_label);
    let phase = state.phase.map_or("preparing", format_phase_label);
    let target = state
        .formula
        .as_deref()
        .or(state.target.as_deref())
        .unwrap_or("request");

    format!(
        "{operation}: {phase} {target} [{count} done]",
        count = state.completed_formulae
    )
}

fn format_operation_label(operation: &str) -> &'static str {
    match operation {
        "install" | "install-plan" => "install",
        "update" => "update",
        "upgrade" | "upgrade-plan" | "upgrade-discovery" => "upgrade",
        "outdated" => "outdated",
        _ => "work",
    }
}

fn format_phase_label(phase: &str) -> &'static str {
    match phase {
        "resolve-install-list" => "resolving",
        "resolve-methods" | "resolve-install-method" => "planning",
        "collect-upgrade-candidates" => "discovering",
        "fetch-formula-index" => "fetching index",
        "persist-formula-index" => "writing index",
        "plan-execution" => "planning execution",
        "prefetch-payload" => "prefetching",
        "check-blob-store" => "checking cache",
        "download-bottle" => "downloading bottle",
        "store-bottle-blob" => "storing bottle",
        "extract-bottle" => "extracting bottle",
        "materialize-payload" => "materializing",
        "defer-to-finalize" => "queueing finalize",
        "build-from-source" => "building from source",
        "refresh-opt-link" => "refreshing links",
        "post-install" => "post-install",
        "fetch-post-install-source" => "loading post-install source",
        "run-post-install" => "running post-install",
        "finalize-install" => "finalizing",
        "unlink-old-keg" => "unlinking old keg",
        "install-target-version" => "installing target version",
        "discover-installed-kegs" => "scanning installed kegs",
        "check-post-install-viability" => "checking post-install",
        "download-source-archive" => "downloading source",
        "extract-source-archive" => "extracting source",
        "persist-source-archive" => "writing source archive",
        _ => "working",
    }
}

fn format_phase_line(operation: &str, phase: &str, target: &str) -> String {
    format!(
        "{}: {} {target}",
        format_operation_label(operation),
        format_phase_label(phase),
    )
}

fn format_warning_line(operation: &str, target: &str, message: &str) -> String {
    format!(
        "warning: {} {target}: {message}",
        format_operation_label(operation)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_status_line() {
        let mut snapshot = RenderSnapshot::default();
        update_snapshot(
            &mut snapshot,
            &ProgressEvent::OperationStarted {
                operation: "install",
                target: "jq,wget".to_owned(),
            },
        );
        update_snapshot(
            &mut snapshot,
            &ProgressEvent::PhaseStarted {
                operation: "install",
                phase: "download-bottle",
                target: "jq".to_owned(),
            },
        );
        update_snapshot(
            &mut snapshot,
            &ProgressEvent::FormulaCompleted {
                operation: "install",
                name: "jq".to_owned(),
            },
        );

        let rendered = render_status_line(&snapshot);
        assert!(rendered.contains("install"));
        assert!(rendered.contains("downloading bottle"));
        assert!(rendered.contains("jq"));
        assert!(rendered.contains("[1 done]"));
    }

    #[test]
    fn test_format_warning_line() {
        let rendered = format_warning_line(
            "upgrade",
            "jq",
            "upgrade failed, restoring previous version",
        );
        assert_eq!(
            rendered,
            "warning: upgrade jq: upgrade failed, restoring previous version"
        );
    }

    #[test]
    fn test_plain_renderer_verbose_phase_line() {
        let renderer = ProgressRenderer::new(Verbosity::Verbose, RenderMode::Plain);
        renderer.handle_event(&ProgressEvent::PhaseStarted {
            operation: "update",
            phase: "fetch-formula-index",
            target: "formula-index".to_owned(),
        });
        let snapshot = renderer
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(snapshot.phase, Some("fetch-formula-index"));
        drop(snapshot);
    }
}
