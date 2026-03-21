use std::fmt;

use crate::{CellarType, FormulaName};

/// Errors that can occur in formula operations.
#[derive(Debug, thiserror::Error)]
pub enum FormulaError {
    /// The requested formula was not found.
    #[error("formula not found: {name}")]
    NotFound {
        /// Name of the missing formula.
        name: FormulaName,
    },

    /// The formula is not supported for installation via brewdock.
    #[error("formula {name} is not supported: {reason}")]
    Unsupported {
        /// Name of the unsupported formula.
        name: FormulaName,
        /// Reason the formula is unsupported.
        reason: UnsupportedReason,
    },

    /// An HTTP request failed.
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// JSON parsing failed.
    #[error("JSON parse error: {0}")]
    Parse(#[from] serde_json::Error),

    /// The raw Ruby source path is invalid.
    #[error("invalid ruby source path: {path}")]
    InvalidRubySourcePath {
        /// The invalid raw source path.
        path: String,
    },

    /// A dependency cycle was detected.
    #[error("cyclic dependency: {0}")]
    CyclicDependency(DependencyCycle),
}

/// Reason a formula is not supported.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnsupportedReason {
    /// The formula is disabled upstream.
    #[error("formula is disabled")]
    Disabled,

    /// No pre-built bottle is available.
    #[error("no bottle available")]
    NoBottle,

    /// The formula defines a `post_install` hook that execution cannot handle yet.
    #[error("has post_install hook")]
    PostInstallDefined,

    /// The formula has a `pour_bottle_only_if` restriction.
    #[error("has pour_bottle_only_if restriction")]
    PourBottleRestricted,

    /// No bottle exists for the specified platform tag.
    #[error("no bottle for platform {0}")]
    NoBottleForTag(String),

    /// The formula requires source fallback execution.
    #[error("requires source build")]
    SourceBuildRequired,

    /// The bottle requires a cellar path incompatible with the current layout.
    #[error("incompatible cellar {0}")]
    IncompatibleCellar(CellarType),
}

/// A dependency cycle represented as a list of formula names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyCycle(Vec<String>);

impl DependencyCycle {
    /// Creates a new dependency cycle from a list of formula names.
    #[must_use]
    pub const fn new(cycle: Vec<String>) -> Self {
        Self(cycle)
    }

    /// Returns the formula names in the cycle.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.0
    }
}

impl fmt::Display for DependencyCycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, name) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(" -> ")?;
            }
            f.write_str(name)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_cycle_display() {
        let cycle = DependencyCycle::new(vec![
            "a".to_owned(),
            "b".to_owned(),
            "c".to_owned(),
            "a".to_owned(),
        ]);
        assert_eq!(cycle.to_string(), "a -> b -> c -> a");
    }

    #[test]
    fn test_unsupported_reason_display() {
        assert_eq!(
            UnsupportedReason::Disabled.to_string(),
            "formula is disabled"
        );
        assert_eq!(
            UnsupportedReason::NoBottleForTag("arm64_sequoia".to_owned()).to_string(),
            "no bottle for platform arm64_sequoia"
        );
        assert_eq!(
            UnsupportedReason::SourceBuildRequired.to_string(),
            "requires source build"
        );
    }

    #[test]
    fn test_incompatible_cellar_display() {
        assert_eq!(
            UnsupportedReason::IncompatibleCellar(CellarType::Path("/usr/local/Cellar".to_owned()))
                .to_string(),
            "incompatible cellar /usr/local/Cellar"
        );
    }
}
