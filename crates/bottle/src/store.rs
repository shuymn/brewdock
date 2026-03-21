use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::error::{BottleError, Sha256Hex};

static TEMP_FILE_NONCE: AtomicUsize = AtomicUsize::new(0);

/// Content-addressable blob store for downloaded bottles.
///
/// Blobs are stored in a sharded directory structure: `<root>/<sha256[0:2]>/<sha256>`.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    /// Creates a new store rooted at the given directory.
    ///
    /// The directory is created lazily on the first [`put`](Self::put) call.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Returns whether a blob with the given SHA256 exists in the store.
    ///
    /// # Errors
    ///
    /// Returns [`BottleError::InvalidSha256`] if `sha256` is not a valid digest.
    pub fn has(&self, sha256: &str) -> Result<bool, BottleError> {
        Ok(self.blob_path(sha256)?.exists())
    }

    /// Returns the filesystem path for a blob with the given SHA256.
    ///
    /// # Errors
    ///
    /// Returns [`BottleError::InvalidSha256`] if `sha256` is not a valid digest.
    pub fn blob_path(&self, sha256: &str) -> Result<PathBuf, BottleError> {
        let sha256 = Sha256Hex::parse(sha256)?;
        let shard = &sha256.as_str()[..2];
        Ok(self.root.join(shard).join(sha256.as_str()))
    }

    /// Writes data to the store under the given SHA256 key.
    ///
    /// Creates parent directories as needed. Overwrites any existing blob
    /// with the same key (idempotent for correct content-addressed data).
    ///
    /// # Errors
    ///
    /// Returns [`BottleError::InvalidSha256`] if `sha256` is not a valid digest.
    /// Returns [`BottleError::Io`] if directory creation or file writing fails.
    pub fn put(&self, sha256: &str, data: &[u8]) -> Result<(), BottleError> {
        let path = self.blob_path(sha256)?;
        if let Some(parent) = path.parent() {
            ensure_directory_is_not_symlink(&self.root)?;
            if parent != self.root {
                std::fs::create_dir_all(parent)?;
                ensure_directory_is_not_symlink(parent)?;
            }
        }

        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(std::io::Error::other(format!(
                    "blob path is a symlink: {}",
                    path.display()
                ))
                .into());
            }
            Ok(metadata) if metadata.is_file() => return Ok(()),
            Ok(_) => {
                return Err(std::io::Error::other(format!(
                    "blob path is not a regular file: {}",
                    path.display()
                ))
                .into());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let temp_path = make_temp_blob_path(&path);
        let write_result = write_temp_blob(&temp_path, data).and_then(|()| {
            std::fs::rename(&temp_path, &path)?;
            Ok(())
        });

        if write_result.is_err() {
            let _ = std::fs::remove_file(&temp_path);
        }

        write_result?;
        Ok(())
    }
}

fn ensure_directory_is_not_symlink(path: &Path) -> Result<(), BottleError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(std::io::Error::other(format!(
            "directory path is a symlink: {}",
            path.display()
        ))
        .into()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(path)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn make_temp_blob_path(path: &Path) -> PathBuf {
    let nonce = TEMP_FILE_NONCE.fetch_add(1, Ordering::Relaxed);
    path.with_extension(format!("tmp-{nonce}"))
}

fn write_temp_blob(path: &Path, data: &[u8]) -> Result<(), std::io::Error> {
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_blob_store_has_returns_false_for_missing() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = BlobStore::new(dir.path());
        assert!(!store.has("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890")?);
        Ok(())
    }

    #[test]
    fn test_blob_store_put_and_has() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = BlobStore::new(dir.path());
        let sha = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";

        assert!(!store.has(sha)?);
        store.put(sha, b"bottle data")?;
        assert!(store.has(sha)?);
        Ok(())
    }

    #[test]
    fn test_blob_store_blob_path_is_sharded() -> Result<(), BottleError> {
        let store = BlobStore::new(Path::new("/tmp/store"));
        let sha = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let path = store.blob_path(sha)?;
        assert_eq!(path, PathBuf::from(format!("/tmp/store/ab/{sha}")));
        Ok(())
    }

    #[test]
    fn test_blob_store_put_creates_directories() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = BlobStore::new(dir.path());
        let sha = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

        store.put(sha, b"data")?;
        let path = store.blob_path(sha)?;
        assert!(path.exists());
        assert_eq!(std::fs::read(&path)?, b"data");
        Ok(())
    }

    #[test]
    fn test_blob_store_put_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = BlobStore::new(dir.path());
        let sha = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";

        store.put(sha, b"data")?;
        store.put(sha, b"data")?;
        assert!(store.has(sha)?);
        Ok(())
    }

    #[test]
    fn test_blob_store_rejects_short_digest() {
        let store = BlobStore::new(Path::new("/tmp/store"));
        let result = store.blob_path("abc");
        assert!(matches!(result, Err(BottleError::InvalidSha256 { .. })));
    }

    #[test]
    fn test_blob_store_rejects_non_hex_digest() {
        let store = BlobStore::new(Path::new("/tmp/store"));
        let digest = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
        let result = store.has(digest);
        assert!(matches!(result, Err(BottleError::InvalidSha256 { .. })));
    }

    #[test]
    fn test_blob_store_put_is_idempotent_when_blob_already_exists()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = BlobStore::new(dir.path());
        let sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

        store.put(sha, b"first")?;
        store.put(sha, b"second")?;

        let path = store.blob_path(sha)?;
        assert_eq!(std::fs::read(&path)?, b"first");
        Ok(())
    }

    #[test]
    fn test_blob_store_put_rejects_symlinked_shard_directory()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = BlobStore::new(dir.path());
        let sha = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let blob_path = store.blob_path(sha)?;
        let shard = blob_path
            .parent()
            .ok_or_else(|| std::io::Error::other("blob path should have a parent directory"))?;
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside)?;
        std::os::unix::fs::symlink(&outside, shard)?;

        let result = store.put(sha, b"data");

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_blob_store_put_does_not_follow_existing_symlink()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = BlobStore::new(dir.path());
        let sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let blob_path = store.blob_path(sha)?;

        let outside = dir.path().join("outside.txt");
        std::fs::write(&outside, "before")?;
        std::fs::create_dir_all(
            blob_path
                .parent()
                .ok_or("blob path should have a parent directory")?,
        )?;
        std::os::unix::fs::symlink(&outside, &blob_path)?;

        let result = store.put(sha, b"after");

        assert!(result.is_err(), "blob store must reject symlink targets");
        assert_eq!(std::fs::read_to_string(&outside)?, "before");
        Ok(())
    }
}
