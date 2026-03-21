use std::path::Path;

use crate::{error::CellarError, fs, link::relative_from_to};

/// Copies extracted bottle contents to the Cellar and creates the `opt/<name>` symlink.
///
/// `source` is the extracted bottle directory containing the formula's files.
/// `keg_path` is the target directory (e.g., `Cellar/<name>/<version>/`).
/// `opt_dir` is the opt directory (e.g., `prefix/opt/`).
/// `name` is the formula name used for the opt symlink.
///
/// # Errors
///
/// Returns [`CellarError::Io`] if copying or symlink creation fails.
pub fn materialize(
    source: &Path,
    keg_path: &Path,
    opt_dir: &Path,
    name: &str,
) -> Result<(), CellarError> {
    let keg_parent = keg_path
        .parent()
        .ok_or_else(|| CellarError::MissingParentDirectory {
            path: keg_path.to_owned(),
        })?;
    std::fs::create_dir_all(keg_parent)?;

    let version = keg_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| CellarError::InvalidPathComponent {
            path: keg_path.to_owned(),
        })?;
    let temp_keg = keg_parent.join(format!(".{version}.brewdock-tmp"));

    // Clean up stale temp dir from a previous crash.
    if temp_keg.exists() {
        std::fs::remove_dir_all(&temp_keg)?;
    }

    copy_tree(source, &temp_keg)?;

    // Idempotent: remove existing keg before atomic rename.
    if keg_path.exists() {
        std::fs::remove_dir_all(keg_path)?;
    }
    std::fs::rename(&temp_keg, keg_path)?;

    // Atomic opt symlink.
    std::fs::create_dir_all(opt_dir)?;
    let opt_link = opt_dir.join(name);
    let rel_target = relative_from_to(opt_dir, keg_path);
    atomic_symlink_replace(&rel_target, &opt_link)?;

    Ok(())
}

/// Creates a symlink atomically by writing to a temporary path and renaming.
///
/// This avoids a window where the link is absent between `remove_file` and `symlink`.
/// If a stale temporary symlink exists from a previous crash, it is cleaned up first.
///
/// # Errors
///
/// Returns [`CellarError::MissingParentDirectory`] if `link_path` has no parent.
/// Returns [`CellarError::InvalidPathComponent`] if `link_path` has no file name.
/// Returns [`CellarError::Io`] on filesystem failure.
pub fn atomic_symlink_replace(target: &Path, link_path: &Path) -> Result<(), CellarError> {
    let link_dir = link_path
        .parent()
        .ok_or_else(|| CellarError::MissingParentDirectory {
            path: link_path.to_owned(),
        })?;
    let name = link_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| CellarError::InvalidPathComponent {
            path: link_path.to_owned(),
        })?;
    let temp_link = link_dir.join(format!(".{name}.brewdock-tmp"));

    // Clean up stale temp from a previous crash.
    if temp_link.symlink_metadata().is_ok() {
        std::fs::remove_file(&temp_link)?;
    }

    std::os::unix::fs::symlink(target, &temp_link)?;
    std::fs::rename(&temp_link, link_path)?;
    Ok(())
}

/// Copies a directory tree, trying `clonefile(2)` first on macOS, falling back
/// to recursive copy.
fn copy_tree(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    #[cfg(target_os = "macos")]
    if brewdock_sys::clone_path(src, dst).is_ok() {
        tracing::debug!(
            src = %src.display(),
            dst = %dst.display(),
            "clonefile succeeded"
        );
        reject_absolute_symlinks(dst)?;
        return Ok(());
    }
    copy_dir_recursive(src, dst)
}

/// Walks `root` and returns an error if any symlink has an absolute target.
fn reject_absolute_symlinks(root: &Path) -> Result<(), std::io::Error> {
    for path in fs::walk_entries(root)? {
        let meta = path.symlink_metadata()?;
        if meta.is_symlink() {
            let target = std::fs::read_link(&path)?;
            if target.is_absolute() {
                return Err(std::io::Error::other(format!(
                    "absolute symlink in bottle: {} -> {}",
                    path.display(),
                    target.display()
                )));
            }
        }
    }
    Ok(())
}

/// Recursively copies a directory tree from `src` to `dst`, preserving symlinks.
///
/// Symlinks are recreated rather than followed, keeping the keg layout intact.
/// Absolute symlink targets are rejected — bottle entries should always use
/// relative targets.
///
/// If the destination file already exists and is read-only (e.g., from a
/// previous bottle pour), it is made writable before overwriting.
pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if ft.is_symlink() {
            let target = std::fs::read_link(&src_path)?;
            if target.is_absolute() {
                return Err(std::io::Error::other(format!(
                    "absolute symlink in bottle: {} -> {}",
                    src_path.display(),
                    target.display()
                )));
            }
            // Remove stale entry before recreating (idempotent re-pour).
            if dst_path.symlink_metadata().is_ok() {
                std::fs::remove_file(&dst_path)?;
            }
            std::os::unix::fs::symlink(&target, &dst_path)?;
        } else {
            fs::make_writable(&dst_path)?;
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_materialize_copies_files_to_keg() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("source");
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/formula/1.0");
        let opt_dir = prefix.join("opt");

        // Create source files.
        std::fs::create_dir_all(source.join("bin"))?;
        std::fs::write(source.join("bin/tool"), "#!/bin/sh")?;
        std::fs::create_dir_all(source.join("lib"))?;
        std::fs::write(source.join("lib/libfoo.dylib"), "fake-dylib")?;

        materialize(&source, &keg_path, &opt_dir, "formula")?;

        assert_eq!(
            std::fs::read_to_string(keg_path.join("bin/tool"))?,
            "#!/bin/sh"
        );
        assert_eq!(
            std::fs::read_to_string(keg_path.join("lib/libfoo.dylib"))?,
            "fake-dylib"
        );
        Ok(())
    }

    #[test]
    fn test_materialize_creates_opt_symlink() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("source");
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/formula/1.0");
        let opt_dir = prefix.join("opt");

        std::fs::create_dir_all(&source)?;
        std::fs::write(source.join("README"), "hello")?;

        materialize(&source, &keg_path, &opt_dir, "formula")?;

        let opt_link = opt_dir.join("formula");
        assert!(opt_link.is_symlink());

        // The opt symlink should resolve to the keg.
        let resolved = std::fs::canonicalize(&opt_link)?;
        let expected = std::fs::canonicalize(&keg_path)?;
        assert_eq!(resolved, expected);
        Ok(())
    }

    #[test]
    fn test_atomic_symlink_replace_creates_new_link() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("target");
        std::fs::create_dir_all(&target)?;
        let link_path = dir.path().join("link");

        atomic_symlink_replace(&target, &link_path)?;

        assert!(link_path.is_symlink());
        assert_eq!(std::fs::read_link(&link_path)?, target);
        Ok(())
    }

    #[test]
    fn test_atomic_symlink_replace_replaces_existing_link() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let old_target = dir.path().join("old");
        let new_target = dir.path().join("new");
        std::fs::create_dir_all(&old_target)?;
        std::fs::create_dir_all(&new_target)?;
        let link_path = dir.path().join("link");

        std::os::unix::fs::symlink(&old_target, &link_path)?;
        assert_eq!(std::fs::read_link(&link_path)?, old_target);

        atomic_symlink_replace(&new_target, &link_path)?;

        assert!(link_path.is_symlink());
        assert_eq!(std::fs::read_link(&link_path)?, new_target);
        Ok(())
    }

    #[test]
    fn test_atomic_symlink_replace_handles_stale_temp() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("target");
        std::fs::create_dir_all(&target)?;
        let link_path = dir.path().join("link");

        // Create a stale temp symlink.
        let stale_temp = dir.path().join(".link.brewdock-tmp");
        std::os::unix::fs::symlink(Path::new("/nonexistent"), &stale_temp)?;
        assert!(stale_temp.symlink_metadata().is_ok());

        atomic_symlink_replace(&target, &link_path)?;

        assert!(link_path.is_symlink());
        assert!(stale_temp.symlink_metadata().is_err());
        Ok(())
    }

    #[test]
    fn test_materialize_cleans_stale_temp_dir() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("source");
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/formula/1.0");
        let opt_dir = prefix.join("opt");

        std::fs::create_dir_all(&source)?;
        std::fs::write(source.join("file.txt"), "data")?;

        // Create a stale temp keg directory.
        let stale_temp = prefix.join("Cellar/formula/.1.0.brewdock-tmp");
        std::fs::create_dir_all(&stale_temp)?;
        std::fs::write(stale_temp.join("stale.txt"), "old")?;

        materialize(&source, &keg_path, &opt_dir, "formula")?;

        assert!(keg_path.join("file.txt").exists());
        assert!(!stale_temp.exists());
        assert!(opt_dir.join("formula").is_symlink());
        Ok(())
    }

    #[test]
    fn test_materialize_preserves_symlinks() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("source");
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/formula/1.0");
        let opt_dir = prefix.join("opt");

        // Create a regular file and a symlink pointing to it.
        std::fs::create_dir_all(source.join("bin"))?;
        std::fs::write(source.join("bin/tool"), "#!/bin/sh")?;
        std::os::unix::fs::symlink("tool", source.join("bin/tool-link"))?;

        // Create a relative symlink across directories.
        std::fs::create_dir_all(source.join("lib"))?;
        std::fs::write(source.join("lib/libfoo.1.0.dylib"), "fake-dylib")?;
        std::os::unix::fs::symlink("libfoo.1.0.dylib", source.join("lib/libfoo.dylib"))?;

        materialize(&source, &keg_path, &opt_dir, "formula")?;

        // The symlinks should be symlinks in the keg, not regular files.
        let tool_link = keg_path.join("bin/tool-link");
        assert!(tool_link.is_symlink(), "tool-link should be a symlink");
        assert_eq!(
            std::fs::read_link(&tool_link)?,
            std::path::PathBuf::from("tool")
        );

        let lib_link = keg_path.join("lib/libfoo.dylib");
        assert!(lib_link.is_symlink(), "libfoo.dylib should be a symlink");
        assert_eq!(
            std::fs::read_link(&lib_link)?,
            std::path::PathBuf::from("libfoo.1.0.dylib")
        );

        // The symlink should resolve to the correct content.
        assert_eq!(std::fs::read_to_string(&tool_link)?, "#!/bin/sh");
        assert_eq!(std::fs::read_to_string(&lib_link)?, "fake-dylib");
        Ok(())
    }

    #[test]
    fn test_materialize_rejects_absolute_symlinks() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("source");
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/formula/1.0");
        let opt_dir = prefix.join("opt");

        std::fs::create_dir_all(source.join("bin"))?;
        std::os::unix::fs::symlink("/usr/bin/env", source.join("bin/bad-link"))?;

        let result = materialize(&source, &keg_path, &opt_dir, "formula");
        assert!(result.is_err(), "absolute symlink should be rejected");
        Ok(())
    }

    #[test]
    fn test_materialize_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("source");
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/formula/1.0");
        let opt_dir = prefix.join("opt");

        std::fs::create_dir_all(&source)?;
        std::fs::write(source.join("file.txt"), "data")?;

        materialize(&source, &keg_path, &opt_dir, "formula")?;
        materialize(&source, &keg_path, &opt_dir, "formula")?;

        assert!(keg_path.join("file.txt").exists());
        assert!(opt_dir.join("formula").is_symlink());
        Ok(())
    }
}
