use std::path::{Path, PathBuf};

use crate::error::BottleError;

/// Content-addressable blob store for downloaded bottles.
///
/// Blobs are stored in a sharded directory structure: `<root>/<sha256[0:2]>/<sha256>`.
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
    #[must_use]
    pub fn has(&self, sha256: &str) -> bool {
        self.blob_path(sha256).exists()
    }

    /// Returns the filesystem path for a blob with the given SHA256.
    #[must_use]
    pub fn blob_path(&self, sha256: &str) -> PathBuf {
        let shard = &sha256[..2];
        self.root.join(shard).join(sha256)
    }

    /// Writes data to the store under the given SHA256 key.
    ///
    /// Creates parent directories as needed. Overwrites any existing blob
    /// with the same key (idempotent for correct content-addressed data).
    ///
    /// # Errors
    ///
    /// Returns [`BottleError::Io`] if directory creation or file writing fails.
    pub fn put(&self, sha256: &str, data: &[u8]) -> Result<(), BottleError> {
        let path = self.blob_path(sha256);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, data)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_blob_store_has_returns_false_for_missing() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = BlobStore::new(dir.path());
        assert!(!store.has("abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"));
        Ok(())
    }

    #[test]
    fn test_blob_store_put_and_has() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = BlobStore::new(dir.path());
        let sha = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";

        assert!(!store.has(sha));
        store.put(sha, b"bottle data")?;
        assert!(store.has(sha));
        Ok(())
    }

    #[test]
    fn test_blob_store_blob_path_is_sharded() {
        let store = BlobStore::new(Path::new("/tmp/store"));
        let sha = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let path = store.blob_path(sha);
        assert_eq!(path, PathBuf::from(format!("/tmp/store/ab/{sha}")));
    }

    #[test]
    fn test_blob_store_put_creates_directories() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let store = BlobStore::new(dir.path());
        let sha = "ff00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

        store.put(sha, b"data")?;
        let path = store.blob_path(sha);
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
        assert!(store.has(sha));
        Ok(())
    }
}
