#![warn(clippy::pedantic, clippy::nursery)]

//! Cellar materialization, receipt, linking, and state management for brewdock.

pub mod error;
pub mod link;
pub mod materialize;
pub mod receipt;
pub mod state;

pub use error::CellarError;
pub use link::{link, unlink};
pub use materialize::materialize;
pub use receipt::{
    InstallReceipt, ReceiptDependency, ReceiptSource, ReceiptSourceVersions, write_receipt,
};
pub use state::{InstallRecord, StateDb};
