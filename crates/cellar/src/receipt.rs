use std::path::Path;

use serde::Serialize;

use crate::error::CellarError;

/// A Homebrew-compatible install receipt.
///
/// Written as `INSTALL_RECEIPT.json` in the keg directory.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[allow(clippy::struct_excessive_bools)] // Matches Homebrew's JSON schema
pub struct InstallReceipt {
    /// Version of the tool that performed the installation.
    pub homebrew_version: String,
    /// Build options used (always empty for bottle installs).
    pub used_options: Vec<String>,
    /// Available but unused build options (always empty for bottle installs).
    pub unused_options: Vec<String>,
    /// Whether the formula was built as a bottle.
    pub built_as_bottle: bool,
    /// Whether the formula was poured from a pre-built bottle.
    pub poured_from_bottle: bool,
    /// Whether the formula was installed as a dependency of another formula.
    pub installed_as_dependency: bool,
    /// Whether the user explicitly requested this formula.
    pub installed_on_request: bool,
    /// Files changed during installation (always empty for bottle pours).
    pub changed_files: Vec<String>,
    /// Installation timestamp as Unix seconds (float).
    pub time: Option<f64>,
    /// Source modification time (null for bottle installs).
    pub source_modified_time: Option<f64>,
    /// Compiler used (nominal for bottles).
    pub compiler: String,
    /// Formula aliases.
    pub aliases: Vec<String>,
    /// Runtime dependencies with version information.
    pub runtime_dependencies: Vec<ReceiptDependency>,
    /// Formula source information.
    pub source: ReceiptSource,
    /// CPU architecture (e.g., `arm64`).
    pub arch: String,
}

/// Why a formula was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallReason {
    /// The formula was explicitly requested by the user.
    OnRequest,
    /// The formula was installed as a dependency.
    AsDependency,
}

impl InstallReceipt {
    /// Creates a receipt for a bottle installation.
    ///
    /// Fixed fields (`built_as_bottle`, `poured_from_bottle`, etc.) are set to
    /// appropriate defaults for bottle pours.
    #[must_use]
    pub fn for_bottle(
        install_reason: InstallReason,
        time: Option<f64>,
        runtime_dependencies: Vec<ReceiptDependency>,
        source: ReceiptSource,
    ) -> Self {
        Self::new(
            install_reason,
            time,
            runtime_dependencies,
            source,
            ReceiptKind::Bottle,
        )
    }

    /// Creates a receipt for a source installation.
    #[must_use]
    pub fn for_source(
        install_reason: InstallReason,
        time: Option<f64>,
        runtime_dependencies: Vec<ReceiptDependency>,
        source: ReceiptSource,
    ) -> Self {
        Self::new(
            install_reason,
            time,
            runtime_dependencies,
            source,
            ReceiptKind::Source,
        )
    }

    fn new(
        install_reason: InstallReason,
        time: Option<f64>,
        runtime_dependencies: Vec<ReceiptDependency>,
        source: ReceiptSource,
        receipt_kind: ReceiptKind,
    ) -> Self {
        let installed_as_dependency = matches!(install_reason, InstallReason::AsDependency);
        let installed_on_request = matches!(install_reason, InstallReason::OnRequest);
        let (built_as_bottle, poured_from_bottle) = match receipt_kind {
            ReceiptKind::Bottle => (true, true),
            ReceiptKind::Source => (false, false),
        };
        Self {
            homebrew_version: format!("brewdock {}", env!("CARGO_PKG_VERSION")),
            used_options: Vec::new(),
            unused_options: Vec::new(),
            built_as_bottle,
            poured_from_bottle,
            installed_as_dependency,
            installed_on_request,
            changed_files: Vec::new(),
            time,
            source_modified_time: None,
            compiler: "clang".to_owned(),
            aliases: Vec::new(),
            runtime_dependencies,
            source,
            arch: canonical_homebrew_arch(std::env::consts::ARCH).to_owned(),
        }
    }
}

#[must_use]
pub fn canonical_homebrew_arch(arch: &str) -> &str {
    match arch {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        _ => arch,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReceiptKind {
    Bottle,
    Source,
}

/// A runtime dependency entry in the install receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReceiptDependency {
    /// Fully qualified formula name.
    pub full_name: String,
    /// Dependency version.
    pub version: String,
    /// Dependency revision.
    pub revision: u32,
    /// Package version string (version with optional revision suffix).
    pub pkg_version: String,
    /// Whether this dependency was declared directly by the formula.
    pub declared_directly: bool,
}

/// Formula source information in the install receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReceiptSource {
    /// Conventional tap path.
    pub path: String,
    /// Tap name (e.g., `homebrew/core`).
    pub tap: String,
    /// Spec name (always `stable` for bottle installs).
    pub spec: String,
    /// Version info from the formula source.
    pub versions: ReceiptSourceVersions,
}

/// Version information within the formula source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReceiptSourceVersions {
    /// Stable version string.
    pub stable: String,
    /// Head version string (if any).
    pub head: Option<String>,
    /// Version scheme number.
    pub version_scheme: u32,
}

/// Writes an install receipt as `INSTALL_RECEIPT.json` in the keg directory.
///
/// # Errors
///
/// Returns [`CellarError::Json`] if serialization fails.
/// Returns [`CellarError::Io`] if the file cannot be written.
pub fn write_receipt(keg_path: &Path, receipt: &InstallReceipt) -> Result<(), CellarError> {
    let path = keg_path.join("INSTALL_RECEIPT.json");
    let json = serde_json::to_string_pretty(receipt)?;
    std::fs::write(path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_receipt() -> InstallReceipt {
        InstallReceipt::for_bottle(
            InstallReason::OnRequest,
            Some(1_700_000_000.0),
            vec![ReceiptDependency {
                full_name: "oniguruma".to_owned(),
                version: "6.9.9".to_owned(),
                revision: 0,
                pkg_version: "6.9.9".to_owned(),
                declared_directly: true,
            }],
            ReceiptSource {
                path: "@@HOMEBREW_PREFIX@@/Library/Taps/homebrew/homebrew-core/Formula/j/jq.rb"
                    .to_owned(),
                tap: "homebrew/core".to_owned(),
                spec: "stable".to_owned(),
                versions: ReceiptSourceVersions {
                    stable: "1.7".to_owned(),
                    head: None,
                    version_scheme: 0,
                },
            },
        )
    }

    #[test]
    fn test_receipt_json_structure() -> Result<(), Box<dyn std::error::Error>> {
        let receipt = sample_receipt();
        let json_str = serde_json::to_string_pretty(&receipt)?;
        let value: serde_json::Value = serde_json::from_str(&json_str)?;

        assert_eq!(value["poured_from_bottle"].as_bool(), Some(true));
        assert_eq!(value["built_as_bottle"].as_bool(), Some(true));
        assert_eq!(value["installed_on_request"].as_bool(), Some(true));
        assert_eq!(value["installed_as_dependency"].as_bool(), Some(false));
        assert_eq!(value["time"].as_f64(), Some(1_700_000_000.0));
        assert!(value["source_modified_time"].is_null());
        assert!(value["used_options"].as_array().is_some_and(Vec::is_empty));
        assert_eq!(value["source"]["tap"].as_str(), Some("homebrew/core"));
        assert_eq!(value["source"]["spec"].as_str(), Some("stable"));
        assert_eq!(value["source"]["versions"]["stable"].as_str(), Some("1.7"));
        assert_eq!(
            value["arch"].as_str(),
            Some(canonical_homebrew_arch(std::env::consts::ARCH))
        );

        let deps = value["runtime_dependencies"]
            .as_array()
            .ok_or("expected array")?;
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0]["full_name"].as_str(), Some("oniguruma"));
        assert_eq!(deps[0]["version"].as_str(), Some("6.9.9"));
        assert_eq!(deps[0]["declared_directly"].as_bool(), Some(true));
        Ok(())
    }

    #[test]
    fn test_write_receipt_creates_file() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg_path = dir.path().join("Cellar/jq/1.7");
        std::fs::create_dir_all(&keg_path)?;

        write_receipt(&keg_path, &sample_receipt())?;

        let path = keg_path.join("INSTALL_RECEIPT.json");
        assert!(path.exists());

        let content = std::fs::read_to_string(&path)?;
        let value: serde_json::Value = serde_json::from_str(&content)?;
        assert_eq!(value["poured_from_bottle"].as_bool(), Some(true));
        Ok(())
    }

    #[test]
    fn test_receipt_arch_matches_homebrew_canonical_value() {
        let receipt = sample_receipt();
        assert_eq!(
            receipt.arch,
            canonical_homebrew_arch(std::env::consts::ARCH)
        );
    }

    #[test]
    fn test_canonical_homebrew_arch_normalizes_known_values() {
        let cases = [
            ("aarch64", "arm64"),
            ("arm64", "arm64"),
            ("x86_64", "x86_64"),
            ("mips64", "mips64"),
        ];

        for (raw, expected) in cases {
            assert_eq!(canonical_homebrew_arch(raw), expected, "raw arch {raw}");
        }
    }

    #[test]
    fn test_receipt_empty_dependencies() -> Result<(), Box<dyn std::error::Error>> {
        let receipt = InstallReceipt::for_bottle(
            InstallReason::AsDependency,
            None,
            Vec::new(),
            ReceiptSource {
                path: String::new(),
                tap: "homebrew/core".to_owned(),
                spec: "stable".to_owned(),
                versions: ReceiptSourceVersions {
                    stable: "1.0".to_owned(),
                    head: None,
                    version_scheme: 0,
                },
            },
        );
        let json_str = serde_json::to_string(&receipt)?;
        let value: serde_json::Value = serde_json::from_str(&json_str)?;

        assert_eq!(value["installed_as_dependency"].as_bool(), Some(true));
        assert_eq!(value["installed_on_request"].as_bool(), Some(false));
        assert!(value["time"].is_null());
        assert!(
            value["runtime_dependencies"]
                .as_array()
                .is_some_and(Vec::is_empty)
        );
        Ok(())
    }

    #[test]
    fn test_write_receipt_returns_io_error_when_keg_path_is_not_directory()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg_path = dir.path().join("not-a-directory");
        std::fs::write(&keg_path, "file")?;

        let result = write_receipt(&keg_path, &sample_receipt());

        assert!(matches!(result, Err(CellarError::Io(_))));
        Ok(())
    }
}
