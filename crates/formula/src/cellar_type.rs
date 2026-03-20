use std::fmt;

use serde::{
    Deserialize,
    de::{self, Deserializer},
};

/// Cellar type for a bottle, controlling how the bottle is installed.
///
/// In Homebrew's JSON API, this is serialized as:
/// - `":any"` → [`CellarType::Any`]
/// - `":any_skip_relocation"` → [`CellarType::AnySkipRelocation`]
/// - Any other string → [`CellarType::Path`]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CellarType {
    /// Bottle can be installed to any Cellar path.
    Any,
    /// Bottle can be installed to any Cellar path, skipping relocation.
    AnySkipRelocation,
    /// Bottle must be installed to a specific Cellar path.
    Path(String),
}

impl<'de> Deserialize<'de> for CellarType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: &str = de::Deserialize::deserialize(deserializer)?;
        Ok(match s {
            ":any" => Self::Any,
            ":any_skip_relocation" => Self::AnySkipRelocation,
            other => Self::Path(other.to_owned()),
        })
    }
}

impl fmt::Display for CellarType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => f.write_str(":any"),
            Self::AnySkipRelocation => f.write_str(":any_skip_relocation"),
            Self::Path(p) => f.write_str(p),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cellar_type_deserialize_any() -> Result<(), serde_json::Error> {
        let ct: CellarType = serde_json::from_str(r#"":any""#)?;
        assert_eq!(ct, CellarType::Any);
        Ok(())
    }

    #[test]
    fn test_cellar_type_deserialize_any_skip_relocation() -> Result<(), serde_json::Error> {
        let ct: CellarType = serde_json::from_str(r#"":any_skip_relocation""#)?;
        assert_eq!(ct, CellarType::AnySkipRelocation);
        Ok(())
    }

    #[test]
    fn test_cellar_type_deserialize_path() -> Result<(), serde_json::Error> {
        let ct: CellarType = serde_json::from_str(r#""/opt/homebrew/Cellar""#)?;
        assert_eq!(ct, CellarType::Path("/opt/homebrew/Cellar".to_owned()));
        Ok(())
    }

    #[test]
    fn test_cellar_type_display() {
        assert_eq!(CellarType::Any.to_string(), ":any");
        assert_eq!(
            CellarType::AnySkipRelocation.to_string(),
            ":any_skip_relocation"
        );
        assert_eq!(
            CellarType::Path("/opt/homebrew/Cellar".to_owned()).to_string(),
            "/opt/homebrew/Cellar"
        );
    }

    #[test]
    fn test_cellar_type_round_trip() -> Result<(), serde_json::Error> {
        for input in [":any", ":any_skip_relocation", "/custom/path"] {
            let json = format!("\"{input}\"");
            let ct: CellarType = serde_json::from_str(&json)?;
            assert_eq!(ct.to_string(), input);
        }
        Ok(())
    }
}
