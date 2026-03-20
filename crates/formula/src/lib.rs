#![warn(clippy::pedantic, clippy::nursery)]

//! Formula types, API client, and dependency resolution for brewdock.

pub mod api;
pub mod cellar_type;
pub mod error;
pub mod resolve;
pub mod supportability;
pub mod types;

pub use api::{FormulaCache, FormulaRepository, HttpFormulaRepository};
pub use cellar_type::CellarType;
pub use error::FormulaError;
pub use types::Formula;
