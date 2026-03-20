use std::path::{Path, PathBuf};

/// Ensures the file at `path` is writable by the owner.
///
/// No-op if the file does not exist or is already writable.
pub fn make_writable(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => {
            let mut perms = meta.permissions();
            let mode = perms.mode();
            if mode & 0o200 == 0 {
                perms.set_mode(mode | 0o200);
                std::fs::set_permissions(path, perms)?;
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Recursively collects all regular file paths under `dir`.
///
/// Symlinks are **not** followed and not included in the result.
/// Symlinked directories are not descended into.
pub fn walk_files(dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut files = Vec::new();
    walk_files_inner(dir, &mut files)?;
    Ok(files)
}

fn walk_files_inner(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            walk_files_inner(&path, files)?;
        } else if ft.is_file() {
            files.push(path);
        }
        // Symlinks skipped — they point to other files handled directly.
    }
    Ok(())
}
