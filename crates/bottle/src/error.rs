use std::fmt;

/// Canonical SHA256 hex digest.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha256Hex(String);

impl Sha256Hex {
    /// Parses and validates a SHA256 hex digest.
    ///
    /// # Errors
    ///
    /// Returns [`BottleError::InvalidSha256`] if the digest is not 64 hex characters.
    pub fn parse(value: &str) -> Result<Self, BottleError> {
        let is_valid = value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
        if !is_valid {
            return Err(BottleError::InvalidSha256 {
                value: value.to_owned(),
            });
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    /// Returns the digest as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Errors that can occur in bottle operations.
#[derive(Debug, thiserror::Error)]
pub enum BottleError {
    /// The provided SHA256 digest is invalid.
    #[error("invalid sha256 digest: {value}")]
    InvalidSha256 {
        /// The invalid digest string.
        value: String,
    },

    /// SHA256 checksum verification failed.
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Expected hex digest.
        expected: Sha256Hex,
        /// Actual computed hex digest.
        actual: Sha256Hex,
    },

    /// An HTTP download failed.
    #[error("download failed: {0}")]
    Download(#[from] reqwest::Error),

    /// Registry authentication failed.
    #[error("registry auth failed: {0}")]
    Auth(String),

    /// A filesystem I/O operation failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}
