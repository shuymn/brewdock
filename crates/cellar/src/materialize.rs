use std::path::{Path, PathBuf};

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

/// Prefix-visible paths created from `.bottle/etc` and `.bottle/var`.
#[derive(Debug, Default)]
pub struct BottlePrefixTransaction {
    created_paths: Vec<PathBuf>,
}

impl BottlePrefixTransaction {
    /// Records a path that should be removed on rollback.
    fn record(&mut self, path: PathBuf) {
        self.created_paths.push(path);
    }

    /// Commits the transaction.
    pub fn commit(self) {}

    /// Removes any prefix-visible paths created by the transaction.
    ///
    /// Paths are removed in reverse creation order so files/symlinks disappear
    /// before the parent directories we created to hold them.
    ///
    /// # Errors
    ///
    /// Returns an error if a file or directory cannot be removed.
    pub fn rollback(self) -> Result<(), CellarError> {
        for path in self.created_paths.into_iter().rev() {
            match path.symlink_metadata() {
                Ok(metadata) if metadata.is_dir() => {
                    std::fs::remove_dir(&path)?;
                }
                Ok(_) => {
                    std::fs::remove_file(&path)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(CellarError::from(error)),
            }
        }
        Ok(())
    }
}

/// Installs Homebrew bottle-managed `etc` and `var` entries into the prefix.
///
/// Relative symlinks are preserved, but destinations remain constrained to
/// `prefix/etc` and `prefix/var`. This matches Homebrew's `.bottle` install
/// behavior without turning it into a generic escape hatch.
///
/// If any step fails after prefix paths were created, those paths are removed
/// before the error is returned (same paths as [`BottlePrefixTransaction::rollback`]).
///
/// # Errors
///
/// Returns an error if any file operation fails or a bottle path escapes the
/// prefix root.
pub fn install_bottle_etc_var(
    keg_path: &Path,
    prefix: &Path,
) -> Result<BottlePrefixTransaction, CellarError> {
    let bottle_root = keg_path.join(".bottle");
    if !bottle_root.exists() {
        return Ok(BottlePrefixTransaction::default());
    }

    let mut transaction = BottlePrefixTransaction::default();
    let outcome = (|| -> Result<(), CellarError> {
        for subdir in ["etc", "var"] {
            let source_root = bottle_root.join(subdir);
            if source_root.is_dir() {
                install_bottle_subtree(
                    &source_root,
                    &source_root,
                    &prefix.join(subdir),
                    &mut transaction,
                )?;
                remove_empty_dir_if_exists(&source_root)?;
            }
        }
        remove_empty_dir_if_exists(&bottle_root)?;
        Ok(())
    })();

    match outcome {
        Ok(()) => Ok(transaction),
        Err(error) => {
            if let Err(rollback_err) = transaction.rollback() {
                tracing::warn!(
                    ?rollback_err,
                    "failed to rollback bottle prefix transaction after install_bottle_etc_var error"
                );
            }
            Err(error)
        }
    }
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

fn install_bottle_subtree(
    source_root: &Path,
    current_source: &Path,
    dest_root: &Path,
    transaction: &mut BottlePrefixTransaction,
) -> Result<(), CellarError> {
    for entry in std::fs::read_dir(current_source)? {
        let entry = entry?;
        let source_path = entry.path();
        let rel_path = source_path
            .strip_prefix(source_root)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let dest_path = validated_dest_path(dest_root, rel_path)?;
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            ensure_dir_exists(&dest_path, transaction)?;
            install_bottle_subtree(source_root, &source_path, dest_root, transaction)?;
            remove_empty_dir_if_exists(&source_path)?;
            continue;
        }

        if let Some(parent) = dest_path.parent() {
            ensure_dir_exists(parent, transaction)?;
        }

        if file_type.is_symlink() {
            install_bottle_symlink(&source_path, &dest_path, transaction)?;
        } else {
            install_bottle_file(&source_path, &dest_path, transaction)?;
        }
    }
    Ok(())
}

fn validated_dest_path(dest_root: &Path, rel_path: &Path) -> Result<PathBuf, CellarError> {
    let joined = dest_root.join(rel_path);
    let normalized = fs::normalize_absolute_path(&joined).ok_or_else(|| {
        CellarError::UnsupportedPostInstallSyntax {
            message: format!("bottle path escapes prefix root: {}", joined.display()),
        }
    })?;
    if normalized.starts_with(dest_root) {
        return Ok(normalized);
    }
    Err(CellarError::UnsupportedPostInstallSyntax {
        message: format!("bottle path escapes prefix root: {}", normalized.display()),
    })
}

fn ensure_dir_exists(
    dir: &Path,
    transaction: &mut BottlePrefixTransaction,
) -> Result<(), CellarError> {
    if dir.is_dir() {
        return Ok(());
    }
    // Record every new ancestor, not just the leaf, so rollback removes all created levels.
    let mut to_create = Vec::new();
    let mut current = dir;
    while !current.exists() {
        to_create.push(current.to_path_buf());
        match current.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => current = parent,
            _ => break,
        }
    }
    std::fs::create_dir_all(dir)?;
    // Record parents before children so rollback (which iterates reversed)
    // removes children before their parents.
    for path in to_create.into_iter().rev() {
        transaction.record(path);
    }
    Ok(())
}

fn install_bottle_file(
    source_path: &Path,
    dest_path: &Path,
    transaction: &mut BottlePrefixTransaction,
) -> Result<(), CellarError> {
    let (target_path, target_preexisting) =
        choose_install_destination_for_file(source_path, dest_path)?;
    if target_path == dest_path && target_preexisting {
        std::fs::remove_file(source_path)?;
        return Ok(());
    }
    if target_path != dest_path && target_preexisting && files_identical(source_path, &target_path)?
    {
        std::fs::remove_file(source_path)?;
        return Ok(());
    }

    fs::make_writable(&target_path)?;
    std::fs::copy(source_path, &target_path)?;
    if !target_preexisting {
        transaction.record(target_path);
    }
    std::fs::remove_file(source_path)?;
    Ok(())
}

fn install_bottle_symlink(
    source_path: &Path,
    dest_path: &Path,
    transaction: &mut BottlePrefixTransaction,
) -> Result<(), CellarError> {
    let target = std::fs::read_link(source_path)?;
    if target.is_absolute() {
        return Err(CellarError::UnsupportedPostInstallSyntax {
            message: format!(
                "absolute symlink in bottle: {} -> {}",
                source_path.display(),
                target.display()
            ),
        });
    }

    let (target_path, already_correct) =
        choose_install_destination_for_symlink(&target, dest_path)?;
    if already_correct {
        std::fs::remove_file(source_path)?;
        return Ok(());
    }

    let target_preexisting = target_path.symlink_metadata().is_ok();
    if target_preexisting {
        if symlink_target_matches(&target_path, &target)? {
            std::fs::remove_file(source_path)?;
            return Ok(());
        }
        fs::make_writable(&target_path)?;
        std::fs::remove_file(&target_path)?;
    }
    std::os::unix::fs::symlink(&target, &target_path)?;
    if !target_preexisting {
        transaction.record(target_path);
    }
    std::fs::remove_file(source_path)?;
    Ok(())
}

fn choose_install_destination_for_file(
    source_path: &Path,
    dest_path: &Path,
) -> Result<(PathBuf, bool), CellarError> {
    if !dest_path.exists() {
        return Ok((dest_path.to_path_buf(), false));
    }
    if files_identical(source_path, dest_path)? {
        return Ok((dest_path.to_path_buf(), true));
    }
    let default = default_install_path(dest_path);
    let exists = default.exists();
    Ok((default, exists))
}

fn choose_install_destination_for_symlink(
    target: &Path,
    dest_path: &Path,
) -> Result<(PathBuf, bool), CellarError> {
    if dest_path.symlink_metadata().is_err() {
        return Ok((dest_path.to_path_buf(), false));
    }
    if symlink_target_matches(dest_path, target)? {
        return Ok((dest_path.to_path_buf(), true));
    }
    Ok((default_install_path(dest_path), false))
}

fn default_install_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.default", path.display()))
}

fn files_identical(source: &Path, dest: &Path) -> Result<bool, CellarError> {
    match std::fs::read(dest) {
        Ok(dest_bytes) => Ok(std::fs::read(source)? == dest_bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(CellarError::from(error)),
    }
}

fn symlink_target_matches(path: &Path, target: &Path) -> Result<bool, CellarError> {
    match std::fs::read_link(path) {
        Ok(existing_target) => Ok(existing_target == target),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(CellarError::from(error)),
    }
}

fn remove_empty_dir_if_exists(path: &Path) -> Result<(), CellarError> {
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::DirectoryNotEmpty
                    | std::io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(CellarError::from(error)),
    }
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

    #[test]
    fn test_install_bottle_etc_var_moves_files_into_prefix()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/demo/1.0");

        std::fs::create_dir_all(keg_path.join(".bottle/etc/demo"))?;
        std::fs::write(keg_path.join(".bottle/etc/demo/config.toml"), "demo=true\n")?;

        let transaction = install_bottle_etc_var(&keg_path, &prefix)?;

        assert_eq!(
            std::fs::read_to_string(prefix.join("etc/demo/config.toml"))?,
            "demo=true\n"
        );
        assert!(!keg_path.join(".bottle/etc/demo/config.toml").exists());

        transaction.commit();
        Ok(())
    }

    #[test]
    fn test_install_bottle_etc_var_writes_default_for_differing_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/demo/1.0");

        std::fs::create_dir_all(prefix.join("etc/demo"))?;
        std::fs::write(prefix.join("etc/demo/config.toml"), "user=true\n")?;
        std::fs::create_dir_all(keg_path.join(".bottle/etc/demo"))?;
        std::fs::write(keg_path.join(".bottle/etc/demo/config.toml"), "demo=true\n")?;

        let transaction = install_bottle_etc_var(&keg_path, &prefix)?;

        assert_eq!(
            std::fs::read_to_string(prefix.join("etc/demo/config.toml"))?,
            "user=true\n"
        );
        assert_eq!(
            std::fs::read_to_string(prefix.join("etc/demo/config.toml.default"))?,
            "demo=true\n"
        );

        transaction.commit();
        Ok(())
    }

    #[test]
    fn test_install_bottle_etc_var_skips_identical_existing_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/demo/1.0");

        std::fs::create_dir_all(prefix.join("etc/demo"))?;
        std::fs::write(prefix.join("etc/demo/config.toml"), "demo=true\n")?;
        std::fs::create_dir_all(keg_path.join(".bottle/etc/demo"))?;
        std::fs::write(keg_path.join(".bottle/etc/demo/config.toml"), "demo=true\n")?;

        let transaction = install_bottle_etc_var(&keg_path, &prefix)?;

        assert!(!prefix.join("etc/demo/config.toml.default").exists());
        assert_eq!(
            std::fs::read_to_string(prefix.join("etc/demo/config.toml"))?,
            "demo=true\n"
        );

        transaction.commit();
        Ok(())
    }

    #[test]
    fn test_install_bottle_etc_var_preserves_relative_symlinks()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/kafka/4.2.0");

        std::fs::create_dir_all(keg_path.join(".bottle/etc/kafka"))?;
        std::fs::write(
            keg_path.join(".bottle/etc/kafka/server.properties"),
            "logs=1\n",
        )?;
        std::fs::create_dir_all(keg_path.join("libexec"))?;
        std::os::unix::fs::symlink("../../../../etc/kafka", keg_path.join("libexec/config"))?;

        let transaction = install_bottle_etc_var(&keg_path, &prefix)?;

        assert_eq!(
            std::fs::read_link(keg_path.join("libexec/config"))?,
            PathBuf::from("../../../../etc/kafka")
        );
        assert_eq!(
            std::fs::read_to_string(prefix.join("etc/kafka/server.properties"))?,
            "logs=1\n"
        );

        transaction.commit();
        Ok(())
    }

    #[test]
    fn test_install_bottle_etc_var_rolls_back_created_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/demo/1.0");

        std::fs::create_dir_all(keg_path.join(".bottle/var/demo"))?;
        std::fs::write(keg_path.join(".bottle/var/demo/state.txt"), "demo\n")?;

        let transaction = install_bottle_etc_var(&keg_path, &prefix)?;
        transaction.rollback()?;

        assert!(!prefix.join("var/demo/state.txt").exists());
        Ok(())
    }

    #[test]
    fn test_install_bottle_etc_var_rolls_back_prefix_on_error_after_etc_succeeds()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg_path = prefix.join("Cellar/demo/1.0");

        std::fs::create_dir_all(keg_path.join(".bottle/etc/demo"))?;
        std::fs::write(keg_path.join(".bottle/etc/demo/config.toml"), "ok\n")?;
        std::fs::create_dir_all(keg_path.join(".bottle/var"))?;
        std::os::unix::fs::symlink("/abs-target", keg_path.join(".bottle/var/bad"))?;

        let result = install_bottle_etc_var(&keg_path, &prefix);
        assert!(
            result.is_err(),
            "absolute symlink under .bottle/var should fail"
        );

        assert!(!prefix.join("etc/demo/config.toml").exists());
        assert!(!prefix.join("etc/demo").exists());
        assert!(!prefix.join("etc").exists());
        Ok(())
    }
}
