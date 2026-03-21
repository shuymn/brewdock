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
    fn test_cellar_type_round_trip_cases() -> Result<(), serde_json::Error> {
        let cases = [
            (
                r#"":any""#,
                CellarType::Any,
                Path::new("/opt/homebrew/Cellar"),
                true,
            ),
            (
                r#"":any_skip_relocation""#,
                CellarType::AnySkipRelocation,
                Path::new("/usr/local/Cellar"),
                true,
            ),
            (
                r#""/opt/homebrew/Cellar""#,
                CellarType::Path("/opt/homebrew/Cellar".to_owned()),
                Path::new("/opt/homebrew/Cellar"),
                true,
            ),
            (
                r#""/usr/local/Cellar""#,
                CellarType::Path("/usr/local/Cellar".to_owned()),
                Path::new("/opt/homebrew/Cellar"),
                false,
            ),
        ];

        for (json, expected, path, compatible) in cases {
            let actual: CellarType = serde_json::from_str(json)?;
            assert_eq!(actual, expected, "deserialize mismatch for {json}");
            assert_eq!(
                actual.to_string(),
                json.trim_matches('"'),
                "display mismatch for {json}"
            );
            assert_eq!(
                actual.is_compatible(path),
                compatible,
                "compatible mismatch for {json}"
            );
        }
        Ok(())
    }
}
