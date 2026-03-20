use std::path::Path;

use crate::{error::CellarError, link::relative_from_to, util};

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
    copy_dir_recursive(source, keg_path)?;

    std::fs::create_dir_all(opt_dir)?;
    let opt_link = opt_dir.join(name);
    let rel_target = relative_from_to(opt_dir, keg_path);
    atomic_symlink_replace(&rel_target, &opt_link, name)?;

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
/// Returns [`CellarError::Io`] on filesystem failure.
pub fn atomic_symlink_replace(
    target: &Path,
    link_path: &Path,
    name: &str,
) -> Result<(), CellarError> {
    let link_dir = link_path
        .parent()
        .ok_or_else(|| CellarError::MissingParentDirectory {
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

/// Recursively copies a directory tree from `src` to `dst`.
///
/// If the destination file already exists and is read-only (e.g., from a
/// previous bottle pour), it is made writable before overwriting.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            util::make_writable(&dst_path)?;
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

        atomic_symlink_replace(&target, &link_path, "link")?;

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

        atomic_symlink_replace(&new_target, &link_path, "link")?;

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

        atomic_symlink_replace(&target, &link_path, "link")?;

        assert!(link_path.is_symlink());
        assert!(stale_temp.symlink_metadata().is_err());
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
