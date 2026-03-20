/// Errors that can occur in bottle operations.
#[derive(Debug, thiserror::Error)]
pub enum BottleError {
    /// SHA256 checksum verification failed.
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Expected hex digest.
        expected: String,
        /// Actual computed hex digest.
        actual: String,
    },

    /// An HTTP download failed.
    #[error("download failed: {0}")]
    Download(#[from] reqwest::Error),

    /// Registry authentication failed.
    #[error("registry auth failed: {0}")]
    Auth(String),

    /// A filesystem I/O operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
