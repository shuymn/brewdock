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
}
