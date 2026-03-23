/// Errors from static analysis of formula `post_install` blocks.
#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    /// A `post_install` block could not be parsed or contains unsupported syntax.
    #[error("unsupported post_install syntax: {message}")]
    UnsupportedPostInstallSyntax {
        /// Human-readable parser failure detail.
        message: String,
    },

    /// A `test do` block could not be parsed or contains unsupported syntax.
    #[error("unsupported test do syntax: {message}")]
    UnsupportedTestDoSyntax {
        /// Human-readable parser failure detail.
        message: String,
    },
}
