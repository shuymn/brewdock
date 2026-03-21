use std::fmt;

use serde::{Deserialize, Serialize};

/// A formula name used in error context and domain boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FormulaName(String);

impl FormulaName {
    /// Returns the name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FormulaName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for FormulaName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for FormulaName {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for FormulaName {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}
