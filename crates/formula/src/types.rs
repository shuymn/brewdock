use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::cellar_type::CellarType;

/// A Homebrew formula parsed from the JSON API.
///
/// Only fields needed for bottle installation are included.
/// Unknown fields in the JSON are silently ignored by serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Formula {
    /// Formula name (e.g., `jq`).
    pub name: String,

    /// Fully qualified name including tap (e.g., `jq`).
    pub full_name: String,

    /// Version information.
    pub versions: Versions,

    /// Package revision (incremented for packaging changes without version bump).
    #[serde(default)]
    pub revision: u32,

    /// Bottle specifications.
    #[serde(default)]
    pub bottle: BottleSpec,

    /// Conditional pour restriction (if set, brewdock rejects).
    pub pour_bottle_only_if: Option<String>,

    /// Whether the formula is keg-only (not linked into prefix).
    #[serde(default)]
    pub keg_only: bool,

    /// Runtime dependencies (formula names).
    #[serde(default)]
    pub dependencies: Vec<String>,

    /// Whether the formula is disabled upstream.
    #[serde(default)]
    pub disabled: bool,

    /// Whether the formula defines a `post_install` hook.
    #[serde(default)]
    pub post_install_defined: bool,
}

/// Version information for a formula.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Versions {
    /// Stable version string.
    pub stable: String,

    /// Head version string (if any).
    pub head: Option<String>,

    /// Whether a pre-built bottle exists.
    #[serde(default)]
    pub bottle: bool,
}

/// Wrapper for bottle specifications.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BottleSpec {
    /// Stable bottle specification.
    pub stable: Option<BottleStable>,
}

/// Stable bottle details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BottleStable {
    /// Rebuild counter.
    #[serde(default)]
    pub rebuild: u32,

    /// Base URL for bottle downloads.
    pub root_url: String,

    /// Per-platform bottle files, keyed by host tag (e.g., `arm64_sequoia`).
    #[serde(default)]
    pub files: HashMap<String, BottleFile>,
}

/// A single bottle file for a specific platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BottleFile {
    /// Cellar type controlling installation path.
    pub cellar: CellarType,

    /// Direct download URL.
    pub url: String,

    /// Expected SHA-256 checksum.
    pub sha256: String,
}

/// Creates a minimal [`Formula`] for testing.
#[cfg(test)]
pub(crate) fn test_formula(name: &str, deps: &[&str]) -> Formula {
    Formula {
        name: name.to_owned(),
        full_name: name.to_owned(),
        versions: Versions {
            stable: "1.0.0".to_owned(),
            head: None,
            bottle: true,
        },
        revision: 0,
        bottle: BottleSpec {
            stable: Some(BottleStable {
                rebuild: 0,
                root_url: "https://example.com".to_owned(),
                files: HashMap::from([(
                    "arm64_sequoia".to_owned(),
                    BottleFile {
                        cellar: CellarType::Any,
                        url: "https://example.com/bottle.tar.gz".to_owned(),
                        sha256: "deadbeef".to_owned(),
                    },
                )]),
            }),
        },
        pour_bottle_only_if: None,
        keg_only: false,
        dependencies: deps.iter().map(|s| (*s).to_owned()).collect(),
        disabled: false,
        post_install_defined: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_jq_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let json = include_str!("../tests/fixtures/formula/jq.json");
        let formula: Formula = serde_json::from_str(json)?;

        assert_eq!(formula.name, "jq");
        assert_eq!(formula.full_name, "jq");
        assert_eq!(formula.versions.stable, "1.8.1");
        assert!(formula.versions.bottle);
        assert_eq!(formula.revision, 0);
        assert!(!formula.disabled);
        assert!(!formula.post_install_defined);
        assert!(!formula.keg_only);
        assert!(formula.pour_bottle_only_if.is_none());
        assert_eq!(formula.dependencies, vec!["oniguruma"]);

        let stable = formula.bottle.stable.as_ref();
        assert!(stable.is_some());

        let stable = stable.ok_or("missing stable bottle")?;
        assert_eq!(stable.root_url, "https://ghcr.io/v2/homebrew/core");
        assert!(stable.files.contains_key("arm64_sequoia"));

        let arm64 = stable
            .files
            .get("arm64_sequoia")
            .ok_or("missing arm64_sequoia")?;
        assert_eq!(arm64.cellar, CellarType::Any);
        assert!(!arm64.sha256.is_empty());
        assert!(!arm64.url.is_empty());

        Ok(())
    }

    #[test]
    fn test_deserialize_oniguruma_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let json = include_str!("../tests/fixtures/formula/oniguruma.json");
        let formula: Formula = serde_json::from_str(json)?;

        assert_eq!(formula.name, "oniguruma");
        assert!(formula.dependencies.is_empty());
        assert!(formula.versions.bottle);
        assert!(!formula.disabled);

        Ok(())
    }

    #[test]
    fn test_deserialize_linux_cellar_type() -> Result<(), Box<dyn std::error::Error>> {
        let json = include_str!("../tests/fixtures/formula/jq.json");
        let formula: Formula = serde_json::from_str(json)?;

        let stable = formula
            .bottle
            .stable
            .as_ref()
            .ok_or("missing stable bottle")?;
        let linux = stable
            .files
            .get("x86_64_linux")
            .ok_or("missing x86_64_linux")?;
        assert_eq!(linux.cellar, CellarType::AnySkipRelocation);

        Ok(())
    }

    #[test]
    fn test_deserialize_empty_bottle() -> Result<(), serde_json::Error> {
        let json = r#"{
            "name": "test",
            "full_name": "test",
            "versions": { "stable": "1.0", "head": null, "bottle": false },
            "bottle": {},
            "pour_bottle_only_if": null
        }"#;
        let formula: Formula = serde_json::from_str(json)?;
        assert!(formula.bottle.stable.is_none());
        assert!(!formula.versions.bottle);
        Ok(())
    }

    #[test]
    fn test_deserialize_minimal_json() -> Result<(), serde_json::Error> {
        let json = r#"{
            "name": "minimal",
            "full_name": "minimal",
            "versions": { "stable": "0.1", "head": null, "bottle": true },
            "pour_bottle_only_if": null
        }"#;
        let formula: Formula = serde_json::from_str(json)?;
        assert_eq!(formula.name, "minimal");
        assert!(formula.dependencies.is_empty());
        assert_eq!(formula.revision, 0);
        assert!(!formula.disabled);
        Ok(())
    }

    #[test]
    fn test_test_formula_helper() {
        let f = test_formula("jq", &["oniguruma"]);
        assert_eq!(f.name, "jq");
        assert_eq!(f.dependencies, vec!["oniguruma"]);
        assert!(f.bottle.stable.is_some());
    }
}
