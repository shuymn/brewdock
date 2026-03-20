use brewdock_bottle::BottleError;
use brewdock_cellar::CellarError;
use brewdock_formula::FormulaError;

use crate::platform::PlatformError;

/// Top-level error type aggregating all sub-crate errors.
#[derive(Debug, thiserror::Error)]
pub enum BrewdockError {
    /// Platform detection or compatibility error.
    #[error(transparent)]
    Platform(#[from] PlatformError),

    /// I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Formula operation error.
    #[error(transparent)]
    Formula(#[from] FormulaError),

    /// Bottle operation error.
    #[error(transparent)]
    Bottle(#[from] BottleError),

    /// Cellar operation error.
    #[error(transparent)]
    Cellar(#[from] CellarError),
}
