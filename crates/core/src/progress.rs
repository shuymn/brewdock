use std::sync::Arc;

/// Observer interface for user-facing operation progress.
pub trait OperationProgressSink: Send + Sync {
    /// Emits a progress event.
    fn emit(&self, event: ProgressEvent);
}

/// Shared progress sink type used across crate boundaries.
pub type SharedProgressSink = Arc<dyn OperationProgressSink>;

/// User-facing progress events emitted by the orchestrator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressEvent {
    /// An operation started.
    OperationStarted {
        /// Operation identifier.
        operation: &'static str,
        /// Human-readable target label.
        target: String,
    },
    /// An operation finished successfully.
    OperationCompleted {
        /// Operation identifier.
        operation: &'static str,
        /// Human-readable target label.
        target: String,
    },
    /// An operation failed.
    OperationFailed {
        /// Operation identifier.
        operation: &'static str,
        /// Human-readable target label.
        target: String,
        /// Error summary.
        error: String,
    },
    /// A phase started.
    PhaseStarted {
        /// Operation identifier.
        operation: &'static str,
        /// Phase identifier.
        phase: &'static str,
        /// Human-readable target label.
        target: String,
    },
    /// A phase finished successfully.
    PhaseCompleted {
        /// Operation identifier.
        operation: &'static str,
        /// Phase identifier.
        phase: &'static str,
        /// Human-readable target label.
        target: String,
    },
    /// A phase failed.
    PhaseFailed {
        /// Operation identifier.
        operation: &'static str,
        /// Phase identifier.
        phase: &'static str,
        /// Human-readable target label.
        target: String,
        /// Error summary.
        error: String,
    },
    /// Formula-level work started.
    FormulaStarted {
        /// Operation identifier.
        operation: &'static str,
        /// Formula name.
        name: String,
    },
    /// Formula-level work completed.
    FormulaCompleted {
        /// Operation identifier.
        operation: &'static str,
        /// Formula name.
        name: String,
    },
    /// Formula-level work failed.
    FormulaFailed {
        /// Operation identifier.
        operation: &'static str,
        /// Formula name.
        name: String,
        /// Error summary.
        error: String,
    },
    /// User-visible warning.
    Warning {
        /// Operation identifier.
        operation: &'static str,
        /// Human-readable target label.
        target: String,
        /// Warning message.
        message: String,
    },
}

/// Default progress sink that ignores all events.
#[derive(Debug, Default)]
pub struct NoopProgressSink;

impl OperationProgressSink for NoopProgressSink {
    fn emit(&self, _event: ProgressEvent) {}
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, atomic::AtomicUsize};

    use super::*;
    use crate::{
        Layout, Orchestrator,
        testutil::{
            MockDownloader, MockRepo, SHA_A, SHA_C, create_bottle_tar_gz, make_formula,
            setup_installed_keg,
        },
    };

    #[derive(Clone)]
    struct RecordingSink {
        events: Arc<Mutex<Vec<ProgressEvent>>>,
    }

    impl RecordingSink {
        fn shared() -> (SharedProgressSink, Arc<Mutex<Vec<ProgressEvent>>>) {
            let events = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::new(Self {
                events: Arc::clone(&events),
            });
            (sink, events)
        }
    }

    impl OperationProgressSink for RecordingSink {
        fn emit(&self, event: ProgressEvent) {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(event);
        }
    }

    #[tokio::test]
    async fn test_update_emits_operation_and_phase_events() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());
        let formulae = vec![make_formula("jq", "1.7", &[], SHA_A)];
        let (progress_sink, events) = RecordingSink::shared();
        let host_tag = crate::testutil::HOST_TAG.parse()?;
        let orchestrator = Orchestrator::with_progress_sink(
            MockRepo::new(formulae),
            MockDownloader::new(vec![], Arc::new(AtomicUsize::new(0))),
            layout,
            host_tag,
            progress_sink,
        );

        orchestrator.update().await?;

        {
            let events = events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert!(events.iter().any(|event| matches!(
                event,
                ProgressEvent::OperationStarted {
                    operation: "update",
                    ..
                }
            )));
            assert!(events.iter().any(|event| matches!(
                event,
                ProgressEvent::PhaseStarted {
                    operation: "update",
                    phase: "fetch-formula-index",
                    ..
                }
            )));
            assert!(events.iter().any(|event| matches!(
                event,
                ProgressEvent::PhaseCompleted {
                    operation: "update",
                    phase: "persist-formula-index",
                    ..
                }
            )));
            assert!(events.iter().any(|event| matches!(
                event,
                ProgressEvent::OperationCompleted {
                    operation: "update",
                    ..
                }
            )));
            drop(events);
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_install_emits_formula_events() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());
        let formula = make_formula("jq", "1.7", &[], SHA_A);
        let tar = create_bottle_tar_gz("jq", "1.7", &[("bin/jq", b"#!/bin/sh")])?;
        let (progress_sink, events) = RecordingSink::shared();
        let host_tag = crate::testutil::HOST_TAG.parse()?;
        let orchestrator = Orchestrator::with_progress_sink(
            MockRepo::new(vec![formula]),
            MockDownloader::new(vec![(SHA_A, tar)], Arc::new(AtomicUsize::new(0))),
            layout,
            host_tag,
            progress_sink,
        );

        orchestrator.install(&["jq"]).await?;

        {
            let events = events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert!(events.iter().any(|event| matches!(
                event,
                ProgressEvent::FormulaStarted {
                    operation: "install",
                    name
                } if name == "jq"
            )));
            assert!(events.iter().any(|event| matches!(
                event,
                ProgressEvent::FormulaCompleted {
                    operation: "install",
                    name
                } if name == "jq"
            )));
            drop(events);
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_upgrade_failure_emits_warning_and_operation_failed()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let layout = Layout::with_root(dir.path());
        setup_installed_keg(&layout, "jq", "1.0", true)?;
        let formula = make_formula("jq", "2.0", &[], SHA_C);
        let (progress_sink, events) = RecordingSink::shared();
        let host_tag = crate::testutil::HOST_TAG.parse()?;
        let orchestrator = Orchestrator::with_progress_sink(
            MockRepo::new(vec![formula]),
            MockDownloader::new(vec![], Arc::new(AtomicUsize::new(0))),
            layout,
            host_tag,
            progress_sink,
        );

        let result = orchestrator.upgrade(&["jq"]).await;
        assert!(result.is_err());

        {
            let events = events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            assert!(events.iter().any(|event| matches!(
                event,
                ProgressEvent::Warning {
                    operation: "upgrade",
                    message,
                    ..
                } if message == "upgrade failed, restoring previous version"
            )));
            assert!(events.iter().any(|event| matches!(
                event,
                ProgressEvent::OperationFailed {
                    operation: "upgrade",
                    ..
                }
            )));
            drop(events);
        }
        Ok(())
    }
}
