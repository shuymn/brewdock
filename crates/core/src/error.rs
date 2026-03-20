pub use brewdock_bottle::BottleError;
pub use brewdock_cellar::CellarError;
pub use brewdock_formula::FormulaError;

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

    /// Source build planning or execution error.
    #[error(transparent)]
    SourceBuild(#[from] SourceBuildError),
}

/// Source build planning or execution failures.
#[derive(Debug, thiserror::Error)]
pub enum SourceBuildError {
    /// Formula requirements are outside the supported subset.
    #[error("unsupported source requirement: {0}")]
    UnsupportedRequirement(String),

    /// The source URL cannot be fetched by the generic driver.
    #[error("unsupported source archive format: {0}")]
    UnsupportedSourceArchive(String),

    /// Source fallback requires a checksum for verified downloads.
    #[error("missing source checksum for {0}")]
    MissingSourceChecksum(String),

    /// The extracted source tree does not expose a supported build entrypoint.
    #[error("unsupported source build system in {0}")]
    UnsupportedBuildSystem(String),

    /// The downloaded archive could not be mapped to a source root directory.
    #[error("failed to determine source root for {0}")]
    MissingSourceRoot(String),

    /// A source build command failed.
    #[error("source build command failed: {command}: {stderr}")]
    CommandFailed { command: String, stderr: String },
}
