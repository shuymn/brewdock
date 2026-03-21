/// Log-level selection derived from `--verbose` / `--quiet` flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Verbosity {
    /// Debug-level logging.
    Verbose,
    /// Info-level logging (default).
    #[default]
    Normal,
    /// Error-level logging only.
    Quiet,
}

impl Verbosity {
    /// Returns `true` when non-error output should be suppressed.
    pub const fn is_quiet(self) -> bool {
        matches!(self, Self::Quiet)
    }
}
