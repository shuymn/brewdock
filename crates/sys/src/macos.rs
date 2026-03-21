use std::{ffi::CString, path::Path};

/// Flags for `clonefile(2)`.
const CLONE_NOFOLLOW: u32 = 0x0001;

unsafe extern "C" {
    fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32) -> libc::c_int;
}

/// Creates a copy-on-write clone of `source` at `dest` using `clonefile(2)`.
///
/// The destination must not already exist. On APFS, this is a near-instant
/// operation that shares physical storage until either side is modified.
///
/// # Errors
///
/// Returns `std::io::Error` on failure. Common error kinds:
/// - `ENOTSUP` — filesystem does not support clonefile (e.g., HFS+, NFS)
/// - `EXDEV` — source and destination are on different filesystems
/// - `EEXIST` — destination already exists
pub fn clone_path(source: &Path, dest: &Path) -> std::io::Result<()> {
    let src_cstr = path_to_cstring(source)?;
    let dst_cstr = path_to_cstring(dest)?;

    // SAFETY: `clonefile` is a POSIX-like syscall on macOS that takes two
    // null-terminated C strings and a flags integer. Both CStrings are valid
    // for the duration of the call. The function does not retain pointers.
    let ret = unsafe { clonefile(src_cstr.as_ptr(), dst_cstr.as_ptr(), CLONE_NOFOLLOW) };

    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn path_to_cstring(path: &Path) -> std::io::Result<CString> {
    use std::os::unix::ffi::OsStrExt;
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::other("path contains interior null byte"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clonefile_copies_file() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("source.txt");
        let dest = dir.path().join("dest.txt");

        std::fs::write(&source, "hello clone")?;
        clone_path(&source, &dest)?;

        assert_eq!(std::fs::read_to_string(&dest)?, "hello clone");
        Ok(())
    }

    #[test]
    fn test_clonefile_copies_directory() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("src_dir");
        let dest = dir.path().join("dst_dir");

        std::fs::create_dir_all(source.join("sub"))?;
        std::fs::write(source.join("sub/file.txt"), "nested")?;

        clone_path(&source, &dest)?;

        assert_eq!(
            std::fs::read_to_string(dest.join("sub/file.txt"))?,
            "nested"
        );
        Ok(())
    }

    #[test]
    fn test_clonefile_fails_if_dest_exists() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("source.txt");
        let dest = dir.path().join("dest.txt");

        std::fs::write(&source, "hello")?;
        std::fs::write(&dest, "already here")?;

        let result = clone_path(&source, &dest);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_clonefile_preserves_symlinks() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("src_dir");
        let dest = dir.path().join("dst_dir");

        std::fs::create_dir_all(source.join("bin"))?;
        std::fs::write(source.join("bin/tool"), "#!/bin/sh")?;
        std::os::unix::fs::symlink("tool", source.join("bin/tool-link"))?;

        clone_path(&source, &dest)?;

        let link = dest.join("bin/tool-link");
        assert!(link.is_symlink());
        assert_eq!(std::fs::read_link(&link)?, std::path::PathBuf::from("tool"));
        assert_eq!(std::fs::read_to_string(&link)?, "#!/bin/sh");
        Ok(())
    }
}
