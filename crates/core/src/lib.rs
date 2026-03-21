#![warn(clippy::pedantic, clippy::nursery)]

//! Core types and orchestration for brewdock.

pub mod error;
mod finalize;
pub mod layout;
pub mod lock;
pub mod orchestrate;
pub mod platform;
mod source_build;

#[cfg(test)]
mod testutil;

pub use brewdock_bottle::{BottleDownloader, HttpBottleDownloader, Sha256Hex};
pub use brewdock_formula::{FormulaRepository, HttpFormulaRepository, MetadataStore};
pub use error::BrewdockError;
pub use layout::Layout;
pub use lock::FileLock;
pub use orchestrate::{InstallMethod, Orchestrator, PlanEntry, SourceBuildPlan, UpgradePlanEntry};
pub use platform::HostTag;
