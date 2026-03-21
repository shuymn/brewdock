use std::path::{Component, Path, PathBuf};

use crate::{error::CellarError, fs};

/// Directories in a keg that are linked into the Homebrew prefix.
const LINKABLE_DIRS: &[&str] = &["bin", "sbin", "lib", "include", "share", "etc"];

/// Creates relative symlinks from keg directories into the Homebrew prefix.
///
/// For each file in the keg's linkable subdirectories (`bin`, `sbin`, `lib`, `include`,
/// `share`, `etc`), a relative symlink is created at the corresponding prefix location.
///
/// # Errors
///
/// Returns [`CellarError::LinkCollision`] if a target already exists and points to
/// a different keg.
/// Returns [`CellarError::MissingParentDirectory`] if a target path has no parent.
/// Returns [`CellarError::Io`] on filesystem failure.
pub fn link(keg_path: &Path, prefix: &Path) -> Result<(), CellarError> {
    for &dir_name in LINKABLE_DIRS {
        let keg_subdir = keg_path.join(dir_name);
        if !keg_subdir.is_dir() {
            continue;
        }

        let entries = fs::walk_entries(&keg_subdir)?;
        for entry in entries {
            let relative = entry
                .strip_prefix(&keg_subdir)
                .map_err(std::io::Error::other)?;
            let link_path = prefix.join(dir_name).join(relative);

            check_link_collision(&link_path, keg_path)?;

            // Remove existing symlink (same keg) before recreating.
            if link_path.symlink_metadata().is_ok() {
                std::fs::remove_file(&link_path)?;
            }

            if let Some(parent) = link_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            let rel_target = relative_from_to(
                link_path
                    .parent()
                    .ok_or_else(|| CellarError::MissingParentDirectory {
                        path: link_path.clone(),
                    })?,
                &entry,
            );
            std::os::unix::fs::symlink(rel_target, &link_path)?;
        }
    }
    Ok(())
}

/// Removes symlinks previously created by [`link`] and cleans up empty directories.
///
/// Only symlinks that point to files within the given keg are removed.
///
/// # Errors
///
/// Returns [`CellarError::Io`] on filesystem failure.
pub fn unlink(keg_path: &Path, prefix: &Path) -> Result<(), CellarError> {
    for &dir_name in LINKABLE_DIRS {
        let keg_subdir = keg_path.join(dir_name);
        if !keg_subdir.is_dir() {
            continue;
        }

        let entries = fs::walk_entries(&keg_subdir)?;
        for entry in entries {
            let relative = entry
                .strip_prefix(&keg_subdir)
                .map_err(std::io::Error::other)?;
            let link_path = prefix.join(dir_name).join(relative);

            if !link_path.is_symlink() {
                continue;
            }

            // Only remove if the symlink points to our keg.
            let target = std::fs::read_link(&link_path)?;
            if let Some(link_parent) = link_path.parent() {
                let resolved = normalize_path(&link_parent.join(&target));
                if resolved.starts_with(keg_path) {
                    std::fs::remove_file(&link_path)?;
                    let stop_at = prefix.join(dir_name);
                    remove_empty_parents(&link_path, &stop_at);
                }
            }
        }
    }
    Ok(())
}

/// Computes the relative path from `from_dir` to `to_path`.
///
/// Both paths should be absolute.
pub(crate) fn relative_from_to(from_dir: &Path, to_path: &Path) -> PathBuf {
    let from_parts: Vec<_> = from_dir.components().collect();
    let to_parts: Vec<_> = to_path.components().collect();

    let common = from_parts
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut result = PathBuf::new();
    for _ in common..from_parts.len() {
        result.push("..");
    }
    for part in &to_parts[common..] {
        result.push(part);
    }
    result
}

/// Normalizes a path by resolving `.` and `..` components without touching the filesystem.
fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                result.pop();
            }
            Component::CurDir => {}
            c => result.push(c),
        }
    }
    result
}

/// Checks whether creating a symlink at `link_path` would collide with an existing
/// file from a different keg.
fn check_link_collision(link_path: &Path, keg_path: &Path) -> Result<(), CellarError> {
    let metadata = match std::fs::symlink_metadata(link_path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(CellarError::Io(e)),
    };

    if metadata.is_symlink() {
        let target = std::fs::read_link(link_path)?;
        if let Some(link_parent) = link_path.parent() {
            let resolved = normalize_path(&link_parent.join(&target));
            if resolved.starts_with(keg_path) {
                return Ok(());
            }
        }
    }

    Err(CellarError::LinkCollision {
        path: link_path.to_path_buf(),
    })
}

/// Removes empty parent directories up to (but not including) `stop_at`.
fn remove_empty_parents(from: &Path, stop_at: &Path) {
    let mut current = from.parent();
    while let Some(dir) = current {
        if dir == stop_at || !dir.starts_with(stop_at) {
            break;
        }
        // remove_dir only succeeds for empty directories.
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
        current = dir.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_keg(
        dir: &Path,
        files: &[(&str, &str)],
    ) -> Result<(PathBuf, PathBuf), Box<dyn std::error::Error>> {
        let prefix = dir.join("prefix");
        let keg_path = prefix.join("Cellar/formula/1.0");
        for &(path, content) in files {
            let full = keg_path.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full, content)?;
        }
        Ok((prefix, keg_path))
    }

    #[test]
    fn test_link_creates_relative_symlinks() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let (prefix, keg_path) = setup_keg(
            dir.path(),
            &[("bin/tool", "#!/bin/sh"), ("lib/libfoo.dylib", "fake")],
        )?;

        link(&keg_path, &prefix)?;

        let link_path = prefix.join("bin/tool");
        assert!(link_path.is_symlink());
        let target = std::fs::read_link(&link_path)?;
        assert_eq!(target, PathBuf::from("../Cellar/formula/1.0/bin/tool"));

        // Verify the symlink resolves to the correct file.
        assert_eq!(std::fs::read_to_string(&link_path)?, "#!/bin/sh");

        let lib_link = prefix.join("lib/libfoo.dylib");
        assert!(lib_link.is_symlink());
        assert_eq!(std::fs::read_to_string(&lib_link)?, "fake");
        Ok(())
    }

    #[test]
    fn test_link_handles_nested_directories() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let (prefix, keg_path) = setup_keg(dir.path(), &[("share/man/man1/tool.1", "man page")])?;

        link(&keg_path, &prefix)?;

        let link_path = prefix.join("share/man/man1/tool.1");
        assert!(link_path.is_symlink());
        let target = std::fs::read_link(&link_path)?;
        assert_eq!(
            target,
            PathBuf::from("../../../Cellar/formula/1.0/share/man/man1/tool.1")
        );
        assert_eq!(std::fs::read_to_string(&link_path)?, "man page");
        Ok(())
    }

    #[test]
    fn test_link_collision_different_keg() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");

        // Create first formula's keg and link it.
        let keg_a = prefix.join("Cellar/formula_a/1.0");
        std::fs::create_dir_all(keg_a.join("bin"))?;
        std::fs::write(keg_a.join("bin/tool"), "a")?;
        link(&keg_a, &prefix)?;

        // Create second formula's keg with a colliding file.
        let keg_b = prefix.join("Cellar/formula_b/1.0");
        std::fs::create_dir_all(keg_b.join("bin"))?;
        std::fs::write(keg_b.join("bin/tool"), "b")?;

        let result = link(&keg_b, &prefix);
        assert!(
            matches!(result, Err(CellarError::LinkCollision { .. })),
            "expected LinkCollision, got {result:?}"
        );
        Ok(())
    }

    #[test]
    fn test_link_same_keg_overwrites() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let (prefix, keg_path) = setup_keg(dir.path(), &[("bin/tool", "#!/bin/sh")])?;

        link(&keg_path, &prefix)?;
        link(&keg_path, &prefix)?;

        assert!(prefix.join("bin/tool").is_symlink());
        Ok(())
    }

    #[test]
    fn test_unlink_removes_symlinks() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let (prefix, keg_path) = setup_keg(
            dir.path(),
            &[("bin/tool", "#!/bin/sh"), ("lib/libfoo.dylib", "fake")],
        )?;

        link(&keg_path, &prefix)?;
        assert!(prefix.join("bin/tool").is_symlink());

        unlink(&keg_path, &prefix)?;
        assert!(!prefix.join("bin/tool").exists());
        assert!(!prefix.join("lib/libfoo.dylib").exists());
        Ok(())
    }

    #[test]
    fn test_unlink_removes_empty_parent_directories() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let (prefix, keg_path) = setup_keg(dir.path(), &[("share/man/man1/tool.1", "man page")])?;

        link(&keg_path, &prefix)?;
        assert!(prefix.join("share/man/man1/tool.1").is_symlink());

        unlink(&keg_path, &prefix)?;
        assert!(!prefix.join("share/man/man1").exists());
        assert!(!prefix.join("share/man").exists());
        // The top-level linkable dir (share) should remain.
        assert!(prefix.join("share").exists());
        Ok(())
    }

    #[test]
    fn test_link_skips_nonexistent_keg_dirs() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        // Keg only has bin, no lib/share/etc.
        let (prefix, keg_path) = setup_keg(dir.path(), &[("bin/tool", "#!/bin/sh")])?;

        link(&keg_path, &prefix)?;
        assert!(prefix.join("bin/tool").is_symlink());
        assert!(!prefix.join("lib").exists());
        Ok(())
    }

    #[test]
    fn test_link_includes_keg_symlinks() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/formula/1.0");

        // Create a regular file and a symlink in the keg.
        std::fs::create_dir_all(keg_path.join("bin"))?;
        std::fs::write(keg_path.join("bin/tool"), "#!/bin/sh")?;
        std::os::unix::fs::symlink("tool", keg_path.join("bin/tool-link"))?;

        // Create a symlink in lib.
        std::fs::create_dir_all(keg_path.join("lib"))?;
        std::fs::write(keg_path.join("lib/libfoo.1.0.dylib"), "fake")?;
        std::os::unix::fs::symlink("libfoo.1.0.dylib", keg_path.join("lib/libfoo.dylib"))?;

        link(&keg_path, &prefix)?;

        // Both the file and the symlink should have prefix links.
        let tool_link = prefix.join("bin/tool");
        assert!(tool_link.is_symlink(), "tool should be linked");
        let tool_sym_link = prefix.join("bin/tool-link");
        assert!(
            tool_sym_link.is_symlink(),
            "tool-link should also be linked into prefix"
        );

        let lib_link = prefix.join("lib/libfoo.dylib");
        assert!(
            lib_link.is_symlink(),
            "libfoo.dylib symlink should be linked into prefix"
        );
        Ok(())
    }

    #[test]
    fn test_link_treats_symlinked_directory_as_leaf() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/formula/1.0");

        // Create a real directory with a file, and a symlink to that directory.
        std::fs::create_dir_all(keg_path.join("lib/real_dir"))?;
        std::fs::write(keg_path.join("lib/real_dir/file.txt"), "data")?;
        std::os::unix::fs::symlink("real_dir", keg_path.join("lib/link_dir"))?;

        link(&keg_path, &prefix)?;

        // The file inside the real directory should be linked.
        assert!(
            prefix.join("lib/real_dir/file.txt").is_symlink(),
            "file in real dir should be linked"
        );
        // The symlinked directory should be linked as a single symlink (leaf),
        // not expanded into a directory tree.
        let link_dir = prefix.join("lib/link_dir");
        assert!(link_dir.is_symlink(), "link_dir should be a symlink");
        assert!(
            !prefix.join("lib/link_dir/file.txt").is_symlink(),
            "should not descend into symlinked directory"
        );
        Ok(())
    }

    #[test]
    fn test_unlink_removes_keg_symlink_links() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/formula/1.0");

        std::fs::create_dir_all(keg_path.join("bin"))?;
        std::fs::write(keg_path.join("bin/tool"), "#!/bin/sh")?;
        std::os::unix::fs::symlink("tool", keg_path.join("bin/tool-link"))?;

        link(&keg_path, &prefix)?;
        assert!(prefix.join("bin/tool-link").is_symlink());

        unlink(&keg_path, &prefix)?;
        assert!(!prefix.join("bin/tool").exists(), "tool should be unlinked");
        assert!(
            !prefix.join("bin/tool-link").exists(),
            "tool-link should be unlinked"
        );
        Ok(())
    }

    #[test]
    fn test_relative_from_to() {
        assert_eq!(
            relative_from_to(Path::new("/a/b"), Path::new("/a/c/d")),
            PathBuf::from("../c/d")
        );
        assert_eq!(
            relative_from_to(Path::new("/a/b/c"), Path::new("/a/d/e")),
            PathBuf::from("../../d/e")
        );
        assert_eq!(
            relative_from_to(Path::new("/a"), Path::new("/a/b")),
            PathBuf::from("b")
        );
    }

    #[test]
    fn test_link_and_unlink_refuse_hostile_prefix_symlink() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let (prefix, keg_path) = setup_keg(dir.path(), &[("bin/tool", "#!/bin/sh")])?;

        let hostile_target = dir.path().join("outside/tool");
        std::fs::create_dir_all(
            hostile_target
                .parent()
                .ok_or_else(|| std::io::Error::other("missing parent"))?,
        )?;
        std::fs::create_dir_all(prefix.join("bin"))?;
        std::os::unix::fs::symlink(&hostile_target, prefix.join("bin/tool"))?;

        let result = link(&keg_path, &prefix);
        assert!(
            matches!(result, Err(CellarError::LinkCollision { .. })),
            "hostile symlinks in the prefix should be rejected"
        );

        let symlink_target = std::fs::read_link(prefix.join("bin/tool"))?;
        assert_eq!(symlink_target, hostile_target);

        unlink(&keg_path, &prefix)?;
        assert!(
            prefix.join("bin/tool").is_symlink(),
            "unlink should not delete a symlink that resolves outside the keg"
        );
        assert_eq!(std::fs::read_link(prefix.join("bin/tool"))?, hostile_target);
        Ok(())
    }
}
