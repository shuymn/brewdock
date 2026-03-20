use std::{fmt, path::Path};

use serde::{
    Deserialize, Serialize,
    de::{self, Deserializer},
    ser::Serializer,
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

impl CellarType {
    /// Returns whether this cellar type is compatible with the given cellar path.
    ///
    /// - [`CellarType::Any`] and [`CellarType::AnySkipRelocation`] are compatible with any path.
    /// - [`CellarType::Path`] is compatible only when the embedded path matches exactly.
    #[must_use]
    pub fn is_compatible(&self, cellar_path: &Path) -> bool {
        match self {
            Self::Any | Self::AnySkipRelocation => true,
            Self::Path(p) => Path::new(p) == cellar_path,
        }
    }
}

impl Serialize for CellarType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Any => serializer.serialize_str(":any"),
            Self::AnySkipRelocation => serializer.serialize_str(":any_skip_relocation"),
            Self::Path(p) => serializer.serialize_str(p),
        }
    }
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
    fn test_is_compatible_any() {
        assert!(CellarType::Any.is_compatible(Path::new("/opt/homebrew/Cellar")));
        assert!(CellarType::Any.is_compatible(Path::new("/usr/local/Cellar")));
    }

    #[test]
    fn test_is_compatible_any_skip_relocation() {
        assert!(CellarType::AnySkipRelocation.is_compatible(Path::new("/opt/homebrew/Cellar")));
        assert!(CellarType::AnySkipRelocation.is_compatible(Path::new("/usr/local/Cellar")));
    }

    #[test]
    fn test_is_compatible_matching_path() {
        let ct = CellarType::Path("/opt/homebrew/Cellar".to_owned());
        assert!(ct.is_compatible(Path::new("/opt/homebrew/Cellar")));
    }

    #[test]
    fn test_is_compatible_mismatching_path() {
        let ct = CellarType::Path("/usr/local/Cellar".to_owned());
        assert!(!ct.is_compatible(Path::new("/opt/homebrew/Cellar")));
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
