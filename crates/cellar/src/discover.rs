use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{error::CellarError, util::normalize_absolute_path};

/// Minimal metadata read from an existing `INSTALL_RECEIPT.json`.
///
/// Only deserializes the fields needed for install state discovery.
/// Unknown fields in the JSON are silently ignored, making this
/// compatible with both brewdock and Homebrew-generated receipts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ReceiptMetadata {
    /// Whether the user explicitly requested this formula.
    #[serde(default)]
    installed_on_request: bool,
}

/// Represents a formula discovered from Homebrew-visible filesystem state.
///
/// Built by scanning the Cellar directory and reading `INSTALL_RECEIPT.json`
/// from the keg directory that the `opt/<name>` symlink points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledKeg {
    /// Formula name (directory name under Cellar).
    pub name: String,
    /// Package version string (directory name under `Cellar/<name>/`).
    ///
    /// This is the full version string including any revision suffix
    /// (e.g., `1.7` or `3.4.1_1`).
    pub pkg_version: String,
    /// Whether the user explicitly requested this formula.
    pub installed_on_request: bool,
}

/// Finds the installed keg for a specific formula.
///
/// Resolves the `opt/<name>` symlink to determine the active version,
/// then reads `INSTALL_RECEIPT.json` from the keg directory.
///
/// Returns `None` if:
/// - The `opt/<name>` symlink does not exist
/// - The symlink target has no version component
/// - The keg directory does not contain `INSTALL_RECEIPT.json`
///
/// # Errors
///
/// Returns [`CellarError::Io`] if reading the symlink or receipt fails.
/// Returns [`CellarError::Json`] if the receipt cannot be parsed.
pub fn find_installed_keg(
    name: &str,
    cellar: &Path,
    opt_dir: &Path,
) -> Result<Option<InstalledKeg>, CellarError> {
    let opt_link = opt_dir.join(name);

    let target = match std::fs::read_link(&opt_link) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let Some(resolved_target) = normalize_opt_target(&opt_link, &target) else {
        return Ok(None);
    };

    let pkg_version = match resolved_target.file_name().and_then(|n| n.to_str()) {
        Some(v) => v.to_owned(),
        None => return Ok(None),
    };

    let keg_path = cellar.join(name).join(&pkg_version);
    if resolved_target != keg_path {
        return Ok(None);
    }
    let receipt_path = keg_path.join("INSTALL_RECEIPT.json");

    let content = match std::fs::read_to_string(&receipt_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let metadata: ReceiptMetadata = serde_json::from_str(&content)?;

    Ok(Some(InstalledKeg {
        name: name.to_owned(),
        pkg_version,
        installed_on_request: metadata.installed_on_request,
    }))
}

fn normalize_opt_target(opt_link: &Path, target: &Path) -> Option<PathBuf> {
    let joined = if target.is_absolute() {
        target.to_path_buf()
    } else {
        opt_link.parent()?.join(target)
    };
    normalize_absolute_path(&joined)
}

/// Discovers all installed kegs from the Cellar directory.
///
/// Scans each subdirectory of `cellar` and delegates to [`find_installed_keg`]
/// to determine the active version via the `opt/<name>` symlink.
///
/// Kegs without a valid opt symlink or receipt are silently skipped.
///
/// # Errors
///
/// Returns [`CellarError::Io`] if reading the Cellar directory fails.
pub fn discover_installed_kegs(
    cellar: &Path,
    opt_dir: &Path,
) -> Result<Vec<InstalledKeg>, CellarError> {
    if !cellar.is_dir() {
        return Ok(Vec::new());
    }

    let mut kegs = Vec::new();
    for entry in std::fs::read_dir(cellar)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };

        if let Some(keg) = find_installed_keg(name, cellar, opt_dir)? {
            kegs.push(keg);
        }
    }

    kegs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(kegs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        InstallReason, InstallReceipt, ReceiptSource, ReceiptSourceVersions, write_receipt,
    };

    fn setup_keg_with_receipt(
        cellar: &Path,
        opt_dir: &Path,
        name: &str,
        version: &str,
        installed_on_request: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let keg_path = cellar.join(name).join(version);
        std::fs::create_dir_all(&keg_path)?;

        let receipt = InstallReceipt::for_bottle(
            if installed_on_request {
                InstallReason::OnRequest
            } else {
                InstallReason::AsDependency
            },
            Some(1_700_000_000.0),
            Vec::new(),
            ReceiptSource {
                path: String::new(),
                tap: "homebrew/core".to_owned(),
                spec: "stable".to_owned(),
                versions: ReceiptSourceVersions {
                    stable: version.to_owned(),
                    head: None,
                    version_scheme: 0,
                },
            },
        );
        write_receipt(&keg_path, &receipt)?;

        // Create opt symlink.
        std::fs::create_dir_all(opt_dir)?;
        let opt_link = opt_dir.join(name);
        let rel_target = crate::link::relative_from_to(opt_dir, &keg_path);
        crate::atomic_symlink_replace(&rel_target, &opt_link)?;

        Ok(())
    }

    #[test]
    fn test_find_installed_keg_returns_keg_when_present() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let cellar = dir.path().join("Cellar");
        let opt_dir = dir.path().join("opt");

        setup_keg_with_receipt(&cellar, &opt_dir, "jq", "1.7", true)?;

        let result = find_installed_keg("jq", &cellar, &opt_dir)?;
        let keg = result.ok_or("expected keg")?;
        assert_eq!(keg.name, "jq");
        assert_eq!(keg.pkg_version, "1.7");
        assert!(keg.installed_on_request);
        Ok(())
    }

    #[test]
    fn test_find_installed_keg_returns_none_when_not_installed()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let cellar = dir.path().join("Cellar");
        let opt_dir = dir.path().join("opt");
        std::fs::create_dir_all(&cellar)?;
        std::fs::create_dir_all(&opt_dir)?;

        let result = find_installed_keg("jq", &cellar, &opt_dir)?;
        assert_eq!(result, None);
        Ok(())
    }

    #[test]
    fn test_find_installed_keg_returns_none_when_receipt_missing()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let cellar = dir.path().join("Cellar");
        let opt_dir = dir.path().join("opt");

        // Create keg directory without receipt.
        let keg_path = cellar.join("jq").join("1.7");
        std::fs::create_dir_all(&keg_path)?;
        std::fs::create_dir_all(&opt_dir)?;
        let rel_target = crate::link::relative_from_to(&opt_dir, &keg_path);
        crate::atomic_symlink_replace(&rel_target, &opt_dir.join("jq"))?;

        let result = find_installed_keg("jq", &cellar, &opt_dir)?;
        assert_eq!(result, None);
        Ok(())
    }

    #[test]
    fn test_find_installed_keg_reads_dependency_install_reason()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let cellar = dir.path().join("Cellar");
        let opt_dir = dir.path().join("opt");

        setup_keg_with_receipt(&cellar, &opt_dir, "oniguruma", "6.9.9", false)?;

        let keg = find_installed_keg("oniguruma", &cellar, &opt_dir)?.ok_or("expected keg")?;
        assert!(!keg.installed_on_request);
        Ok(())
    }

    #[test]
    fn test_find_installed_keg_handles_version_with_revision()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let cellar = dir.path().join("Cellar");
        let opt_dir = dir.path().join("opt");

        setup_keg_with_receipt(&cellar, &opt_dir, "openssl@3", "3.4.1_1", true)?;

        let keg = find_installed_keg("openssl@3", &cellar, &opt_dir)?.ok_or("expected keg")?;
        assert_eq!(keg.pkg_version, "3.4.1_1");
        Ok(())
    }

    #[test]
    fn test_find_installed_keg_reads_homebrew_generated_receipt()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let cellar = dir.path().join("Cellar");
        let opt_dir = dir.path().join("opt");

        // Write a minimal Homebrew-style receipt with extra fields.
        let keg_path = cellar.join("tree").join("2.2.1");
        std::fs::create_dir_all(&keg_path)?;
        let homebrew_receipt = serde_json::json!({
            "homebrew_version": "4.4.20",
            "used_options": [],
            "unused_options": [],
            "built_as_bottle": true,
            "poured_from_bottle": true,
            "installed_as_dependency": false,
            "installed_on_request": true,
            "changed_files": ["INSTALL_RECEIPT.json"],
            "time": 1_700_000_000,
            "source_modified_time": 1_700_000_000,
            "compiler": "clang",
            "aliases": [],
            "runtime_dependencies": [],
            "source": {
                "path": "@@HOMEBREW_PREFIX@@/Library/Taps/homebrew/homebrew-core/Formula/t/tree.rb",
                "tap": "homebrew/core",
                "tap_git_head": "abc123",
                "spec": "stable",
                "versions": {
                    "stable": "2.2.1",
                    "head": null,
                    "version_scheme": 0
                }
            },
            "arch": "arm64",
            "built_on": {
                "os": "Macintosh",
                "os_version": "macOS 15.3",
                "cpu_family": "dunno",
                "xcode": "16.2",
                "clt": "16.2.0.0.1.1733547573"
            }
        });
        std::fs::write(
            keg_path.join("INSTALL_RECEIPT.json"),
            serde_json::to_string_pretty(&homebrew_receipt)?,
        )?;

        std::fs::create_dir_all(&opt_dir)?;
        let rel_target = crate::link::relative_from_to(&opt_dir, &keg_path);
        crate::atomic_symlink_replace(&rel_target, &opt_dir.join("tree"))?;

        let keg = find_installed_keg("tree", &cellar, &opt_dir)?.ok_or("expected keg")?;
        assert_eq!(keg.name, "tree");
        assert_eq!(keg.pkg_version, "2.2.1");
        assert!(keg.installed_on_request);
        Ok(())
    }

    #[test]
    fn test_find_installed_keg_accepts_absolute_opt_link_with_matching_keg()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let cellar = dir.path().join("Cellar");
        let opt_dir = dir.path().join("opt");
        let keg_path = cellar.join("jq").join("1.7");

        std::fs::create_dir_all(&keg_path)?;
        let receipt = InstallReceipt::for_bottle(
            InstallReason::OnRequest,
            Some(1_700_000_000.0),
            Vec::new(),
            ReceiptSource {
                path: String::new(),
                tap: "homebrew/core".to_owned(),
                spec: "stable".to_owned(),
                versions: ReceiptSourceVersions {
                    stable: "1.7".to_owned(),
                    head: None,
                    version_scheme: 0,
                },
            },
        );
        write_receipt(&keg_path, &receipt)?;
        std::fs::create_dir_all(&opt_dir)?;
        crate::atomic_symlink_replace(&keg_path, &opt_dir.join("jq"))?;

        let keg = find_installed_keg("jq", &cellar, &opt_dir)?.ok_or("expected keg")?;
        assert_eq!(keg.pkg_version, "1.7");
        Ok(())
    }

    #[test]
    fn test_find_installed_keg_rejects_mismatched_opt_symlink_target()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let cellar = dir.path().join("Cellar");
        let opt_dir = dir.path().join("opt");

        let keg_path = cellar.join("jq").join("1.7");
        std::fs::create_dir_all(&keg_path)?;
        let receipt = InstallReceipt::for_bottle(
            InstallReason::OnRequest,
            Some(1_700_000_000.0),
            Vec::new(),
            ReceiptSource {
                path: String::new(),
                tap: "homebrew/core".to_owned(),
                spec: "stable".to_owned(),
                versions: ReceiptSourceVersions {
                    stable: "1.7".to_owned(),
                    head: None,
                    version_scheme: 0,
                },
            },
        );
        write_receipt(&keg_path, &receipt)?;

        let hostile_target = dir.path().join("outside/other/1.7");
        std::fs::create_dir_all(
            hostile_target
                .parent()
                .ok_or_else(|| std::io::Error::other("missing parent"))?,
        )?;
        std::fs::create_dir_all(&opt_dir)?;
        crate::atomic_symlink_replace(&hostile_target, &opt_dir.join("jq"))?;

        let result = find_installed_keg("jq", &cellar, &opt_dir)?;
        assert!(
            result.is_none(),
            "an opt symlink pointing outside the keg tree should not be trusted"
        );
        Ok(())
    }

    #[test]
    fn test_find_installed_keg_rejects_opt_link_outside_formula_directory()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let cellar = dir.path().join("Cellar");
        let opt_dir = dir.path().join("opt");

        let other_keg = cellar.join("other").join("9.9");
        std::fs::create_dir_all(&other_keg)?;
        write_receipt(
            &other_keg,
            &InstallReceipt::for_bottle(
                InstallReason::OnRequest,
                Some(1_700_000_000.0),
                Vec::new(),
                ReceiptSource {
                    path: String::new(),
                    tap: "homebrew/core".to_owned(),
                    spec: "stable".to_owned(),
                    versions: ReceiptSourceVersions {
                        stable: "9.9".to_owned(),
                        head: None,
                        version_scheme: 0,
                    },
                },
            ),
        )?;

        std::fs::create_dir_all(&opt_dir)?;
        std::os::unix::fs::symlink("../Cellar/other/9.9", opt_dir.join("target"))?;

        let found = find_installed_keg("target", &cellar, &opt_dir)?;
        assert_eq!(
            found, None,
            "opt symlink pointing at a different formula directory must be ignored"
        );
        Ok(())
    }

    #[test]
    fn test_discover_installed_kegs_finds_all() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let cellar = dir.path().join("Cellar");
        let opt_dir = dir.path().join("opt");

        setup_keg_with_receipt(&cellar, &opt_dir, "jq", "1.7", true)?;
        setup_keg_with_receipt(&cellar, &opt_dir, "oniguruma", "6.9.9", false)?;
        setup_keg_with_receipt(&cellar, &opt_dir, "ripgrep", "14.1.1", true)?;

        let kegs = discover_installed_kegs(&cellar, &opt_dir)?;
        let names: Vec<_> = kegs.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, vec!["jq", "oniguruma", "ripgrep"]);
        Ok(())
    }

    #[test]
    fn test_discover_installed_kegs_returns_empty_when_cellar_missing()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let cellar = dir.path().join("nonexistent");
        let opt_dir = dir.path().join("opt");

        let kegs = discover_installed_kegs(&cellar, &opt_dir)?;
        assert!(kegs.is_empty());
        Ok(())
    }

    #[test]
    fn test_discover_installed_kegs_skips_kegs_without_receipt()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let cellar = dir.path().join("Cellar");
        let opt_dir = dir.path().join("opt");

        setup_keg_with_receipt(&cellar, &opt_dir, "jq", "1.7", true)?;

        // Create a keg without receipt (interrupted install).
        let broken_keg = cellar.join("broken").join("1.0");
        std::fs::create_dir_all(&broken_keg)?;
        std::fs::create_dir_all(&opt_dir)?;
        let rel_target = crate::link::relative_from_to(&opt_dir, &broken_keg);
        crate::atomic_symlink_replace(&rel_target, &opt_dir.join("broken"))?;

        let kegs = discover_installed_kegs(&cellar, &opt_dir)?;
        assert_eq!(kegs.len(), 1);
        assert_eq!(kegs[0].name, "jq");
        Ok(())
    }
}
