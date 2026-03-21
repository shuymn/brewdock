use std::collections::HashMap;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self},
    ser::SerializeMap,
};

use crate::cellar_type::CellarType;

/// A Homebrew formula parsed from the JSON API.
///
/// Only fields needed for bottle install, source fallback planning, and
/// restricted post-install execution are included.
/// Unknown fields in the JSON are silently ignored by serde.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Path to the Ruby formula source within `homebrew/core`.
    #[serde(default)]
    pub ruby_source_path: Option<String>,

    /// Bottle specifications.
    #[serde(default)]
    pub bottle: BottleSpec,

    /// Source URLs for this formula.
    #[serde(default)]
    pub urls: FormulaUrls,

    /// Conditional pour restriction (if set, brewdock rejects).
    pub pour_bottle_only_if: Option<String>,

    /// Whether the formula is keg-only (not linked into prefix).
    #[serde(default)]
    pub keg_only: bool,

    /// Runtime dependencies (formula names).
    #[serde(default)]
    pub dependencies: Vec<String>,

    /// Build-time dependencies required for source fallback.
    #[serde(default)]
    pub build_dependencies: Vec<String>,

    /// Dependencies provided by macOS and not installed by brewdock.
    #[serde(default)]
    pub uses_from_macos: Vec<MacOsDependency>,

    /// Additional Homebrew requirements.
    #[serde(default)]
    pub requirements: Vec<Requirement>,

    /// Whether the formula is disabled upstream.
    #[serde(default)]
    pub disabled: bool,

    /// Whether the formula defines a `post_install` hook.
    #[serde(default)]
    pub post_install_defined: bool,
}

/// Version information for a formula.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BottleSpec {
    /// Stable bottle specification.
    pub stable: Option<BottleStable>,
}

/// Stable bottle details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BottleFile {
    /// Cellar type controlling installation path.
    pub cellar: CellarType,

    /// Direct download URL.
    pub url: String,

    /// Expected SHA-256 checksum.
    pub sha256: String,
}

/// Source URLs for a formula.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FormulaUrls {
    /// Stable source information.
    #[serde(default)]
    pub stable: Option<StableUrl>,
}

/// Stable source information for a formula.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StableUrl {
    /// Source archive or VCS URL.
    pub url: String,

    /// Expected SHA-256 checksum when provided.
    #[serde(default)]
    pub checksum: Option<String>,
}

/// A dependency that macOS may provide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacOsDependency {
    /// String shorthand such as `"zlib"`.
    Name(String),
    /// Structured entry with a dependency name and usage contexts.
    Detailed(MacOsDependencyDetail),
}

/// Structured `uses_from_macos` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacOsDependencyDetail {
    /// macOS-provided dependency name.
    pub name: String,
    /// Contexts where the dependency is used.
    pub contexts: Vec<String>,
}

/// Homebrew requirement entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Requirement {
    /// String shorthand such as `"xcode"`.
    Name(String),
    /// Structured requirement with at least a name.
    Detailed(NamedEntry),
}

/// Shared structured entry shape for named Homebrew metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedEntry {
    /// Name of the dependency or requirement.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum MacOsDependencyWire {
    Name(String),
    Detailed(HashMap<String, MacOsDependencyContexts>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum MacOsDependencyContexts {
    One(String),
    Many(Vec<String>),
}

const USES_FROM_MACOS_ENTRY_ERROR: &str = "uses_from_macos object must contain exactly one entry";

impl MacOsDependencyContexts {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(context) => vec![context],
            Self::Many(contexts) => contexts,
        }
    }
}

impl Serialize for MacOsDependency {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Name(name) => serializer.serialize_str(name),
            Self::Detailed(detail) => {
                let mut map = serializer.serialize_map(Some(1))?;
                match detail.contexts.as_slice() {
                    [context] => map.serialize_entry(&detail.name, context)?,
                    contexts => map.serialize_entry(&detail.name, contexts)?,
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for MacOsDependency {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = MacOsDependencyWire::deserialize(deserializer)?;
        match wire {
            MacOsDependencyWire::Name(name) => Ok(Self::Name(name)),
            MacOsDependencyWire::Detailed(entries) => Ok(Self::Detailed(
                deserialize_macos_dependency_detail(entries)?,
            )),
        }
    }
}

fn deserialize_macos_dependency_detail<E>(
    entries: HashMap<String, MacOsDependencyContexts>,
) -> Result<MacOsDependencyDetail, E>
where
    E: de::Error,
{
    let mut entries = entries.into_iter();
    let (name, contexts) = entries
        .next()
        .ok_or_else(|| E::custom(USES_FROM_MACOS_ENTRY_ERROR))?;
    if entries.next().is_some() {
        return Err(E::custom(USES_FROM_MACOS_ENTRY_ERROR));
    }

    Ok(MacOsDependencyDetail {
        name,
        contexts: contexts.into_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_formula;

    fn minimal_formula_json(uses_from_macos: &str) -> String {
        format!(
            r#"{{
            "name": "adversarial",
            "full_name": "adversarial",
            "versions": {{ "stable": "1.0.0", "head": null, "bottle": true }},
            "pour_bottle_only_if": null,
            "uses_from_macos": {uses_from_macos}
        }}"#
        )
    }

    fn detailed_dependency(name: &str, contexts: &[&str]) -> MacOsDependency {
        MacOsDependency::Detailed(MacOsDependencyDetail {
            name: name.to_owned(),
            contexts: contexts
                .iter()
                .map(|context| (*context).to_owned())
                .collect(),
        })
    }

    #[test]
    fn test_deserialize_jq_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let json = include_str!("../tests/fixtures/formula/jq.json");
        let formula: Formula = serde_json::from_str(json)?;

        assert_eq!(formula.name, "jq");
        assert_eq!(formula.full_name, "jq");
        assert_eq!(formula.versions.stable, "1.8.1");
        assert!(formula.versions.bottle);
        assert_eq!(formula.revision, 0);
        assert!(formula.ruby_source_path.is_none());
        assert!(!formula.disabled);
        assert!(!formula.post_install_defined);
        assert!(!formula.keg_only);
        assert!(formula.pour_bottle_only_if.is_none());
        assert_eq!(formula.dependencies, vec!["oniguruma"]);
        assert!(formula.build_dependencies.is_empty());
        assert!(formula.uses_from_macos.is_empty());
        assert!(formula.requirements.is_empty());

        let stable = formula.bottle.stable.as_ref();
        assert!(stable.is_some());

        let stable = stable.ok_or("missing stable bottle")?;
        assert_eq!(stable.root_url, "https://ghcr.io/v2/homebrew/core");
        assert!(stable.files.contains_key("arm64_sequoia"));
        assert_eq!(
            formula
                .urls
                .stable
                .as_ref()
                .map(|stable| stable.url.as_str()),
            Some("https://github.com/jqlang/jq/releases/download/jq-1.8.1/jq-1.8.1.tar.gz")
        );
        assert_eq!(
            formula
                .urls
                .stable
                .as_ref()
                .and_then(|stable| stable.checksum.as_deref()),
            Some("2be64e7129cecb11d5906290eba10af694fb9e3e7f9fc208a311dc33ca837eb0")
        );

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
        assert_eq!(
            formula.build_dependencies,
            vec![
                "autoconf".to_owned(),
                "automake".to_owned(),
                "libtool".to_owned()
            ]
        );

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
        assert!(formula.urls.stable.is_none());
        assert!(formula.build_dependencies.is_empty());
        Ok(())
    }

    #[test]
    fn test_test_formula_helper() {
        let f = test_formula("jq", &["oniguruma"]);
        assert_eq!(f.name, "jq");
        assert_eq!(f.dependencies, vec!["oniguruma"]);
        assert!(f.bottle.stable.is_some());
    }

    #[test]
    fn test_deserialize_uses_from_macos_string_form() -> Result<(), serde_json::Error> {
        let dep: MacOsDependency = serde_json::from_str(r#""curl""#)?;
        assert_eq!(dep, MacOsDependency::Name("curl".to_owned()));
        Ok(())
    }

    #[test]
    fn test_deserialize_uses_from_macos_single_context_object() -> Result<(), serde_json::Error> {
        let dep: MacOsDependency = serde_json::from_str(r#"{"zsh":"test"}"#)?;
        assert_eq!(dep, detailed_dependency("zsh", &["test"]));
        Ok(())
    }

    #[test]
    fn test_deserialize_uses_from_macos_multiple_contexts_object() -> Result<(), serde_json::Error>
    {
        let dep: MacOsDependency = serde_json::from_str(r#"{"zsh":["build","test"]}"#)?;
        assert_eq!(dep, detailed_dependency("zsh", &["build", "test"]));
        Ok(())
    }

    #[test]
    fn test_deserialize_uses_from_macos_mixed_array() -> Result<(), serde_json::Error> {
        let deps: Vec<MacOsDependency> = serde_json::from_str(r#"["curl",{"rsync":"build"}]"#)?;
        assert_eq!(
            deps,
            vec![
                MacOsDependency::Name("curl".to_owned()),
                detailed_dependency("rsync", &["build"]),
            ]
        );
        Ok(())
    }

    #[test]
    fn test_deserialize_zsh_completions_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let json = include_str!("../tests/fixtures/formula/zsh-completions.json");
        let formula: Formula = serde_json::from_str(json)?;
        assert_eq!(
            formula.uses_from_macos,
            vec![detailed_dependency("zsh", &["test"])]
        );
        Ok(())
    }

    #[test]
    fn test_deserialize_zsh_fast_syntax_highlighting_fixture()
    -> Result<(), Box<dyn std::error::Error>> {
        let json = include_str!("../tests/fixtures/formula/zsh-fast-syntax-highlighting.json");
        let formula: Formula = serde_json::from_str(json)?;
        assert_eq!(
            formula.uses_from_macos,
            vec![detailed_dependency("zsh", &["build", "test"])]
        );
        Ok(())
    }

    #[test]
    fn test_deserialize_sqlmap_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let json = include_str!("../tests/fixtures/formula/sqlmap.json");
        let formula: Formula = serde_json::from_str(json)?;
        assert_eq!(
            formula.uses_from_macos,
            vec![detailed_dependency("sqlite", &["test"])]
        );
        Ok(())
    }

    #[test]
    fn test_deserialize_semgrep_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let json = include_str!("../tests/fixtures/formula/semgrep.json");
        let formula: Formula = serde_json::from_str(json)?;
        assert_eq!(
            formula.uses_from_macos,
            vec![
                detailed_dependency("rsync", &["build"]),
                MacOsDependency::Name("curl".to_owned()),
            ]
        );
        assert!(
            formula
                .bottle
                .stable
                .as_ref()
                .is_some_and(|stable| stable.files.contains_key("arm64_tahoe"))
        );
        Ok(())
    }

    #[test]
    fn test_deserialize_rejects_empty_uses_from_macos_object() {
        let result: Result<Formula, _> = serde_json::from_str(&minimal_formula_json("[{}]"));
        assert!(
            result.is_err(),
            "empty uses_from_macos object must fail closed"
        );
    }

    #[test]
    fn test_deserialize_rejects_multi_entry_uses_from_macos_object() {
        let result: Result<Formula, _> =
            serde_json::from_str(&minimal_formula_json(r#"[{"zlib":"build","curl":"test"}]"#));
        assert!(
            result.is_err(),
            "multi-entry uses_from_macos object must fail closed"
        );
    }
}
