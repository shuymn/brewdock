#![warn(clippy::pedantic, clippy::nursery)]

//! Core types and orchestration for brewdock.

pub mod error;
pub mod layout;
pub mod lock;
pub mod orchestrate;
pub mod platform;

pub use brewdock_bottle::{BottleDownloader, HttpBottleDownloader};
pub use brewdock_formula::{FormulaRepository, HttpFormulaRepository};
pub use error::BrewdockError;
pub use layout::Layout;
pub use lock::FileLock;
pub use orchestrate::{Orchestrator, PlanEntry, UpgradePlanEntry};
pub use platform::HostTag;
