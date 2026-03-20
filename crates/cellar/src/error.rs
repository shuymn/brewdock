use std::path::PathBuf;

/// Errors that can occur in cellar operations.
#[derive(Debug, thiserror::Error)]
pub enum CellarError {
    /// A filesystem I/O operation failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// A symlink target already exists and points to a different keg.
    #[error("link collision at {path}")]
    LinkCollision {
        /// Path where the collision occurred.
        path: PathBuf,
    },

    /// A required parent directory could not be determined.
    #[error("missing parent directory for {path}")]
    MissingParentDirectory {
        /// Path missing a parent directory.
        path: PathBuf,
    },

    /// A `SQLite` database operation failed.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// JSON serialization or deserialization failed.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// A `post_install` block could not be parsed or contains unsupported syntax.
    #[error("unsupported post_install syntax: {message}")]
    UnsupportedPostInstallSyntax {
        /// Human-readable parser failure detail.
        message: String,
    },

    /// Executing a `post_install` command failed.
    #[error("post_install command failed: {message}")]
    PostInstallCommandFailed {
        /// Human-readable command failure detail.
        message: String,
    },
}
