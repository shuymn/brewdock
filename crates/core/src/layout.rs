use std::path::{Path, PathBuf};

/// Filesystem layout for Homebrew-compatible directory structure.
///
/// All paths are derived from a root directory. In production, the root is `/`,
/// so the Homebrew prefix is `/opt/homebrew`. For testing, use [`Layout::with_root`]
/// to isolate paths under a temporary directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[must_use]
pub struct Layout {
    prefix: PathBuf,
}

impl Layout {
    /// Creates a layout with the production root (`/`).
    ///
    /// The Homebrew prefix will be `/opt/homebrew`.
    pub fn production() -> Self {
        Self {
            prefix: PathBuf::from("/opt/homebrew"),
        }
    }

    /// Creates a layout with a custom root directory.
    ///
    /// The Homebrew prefix will be at `{root}/opt/homebrew`.
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self {
            prefix: root.into().join("opt/homebrew"),
        }
    }

    /// Homebrew prefix directory.
    #[must_use]
    pub fn prefix(&self) -> &Path {
        &self.prefix
    }

    /// Cellar directory (`{prefix}/Cellar`).
    #[must_use]
    pub fn cellar(&self) -> PathBuf {
        self.prefix.join("Cellar")
    }

    /// Opt directory for version-pinned symlinks (`{prefix}/opt`).
    #[must_use]
    pub fn opt_dir(&self) -> PathBuf {
        self.prefix.join("opt")
    }

    /// Bin directory for linked executables (`{prefix}/bin`).
    #[must_use]
    pub fn bin_dir(&self) -> PathBuf {
        self.prefix.join("bin")
    }

    /// brewdock state directory (`{prefix}/var/brewdock`).
    #[must_use]
    pub fn var_brewdock(&self) -> PathBuf {
        self.prefix.join("var/brewdock")
    }

    /// Formula cache directory (`{prefix}/var/brewdock/cache`).
    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.var_brewdock().join("cache")
    }

    /// CAS blob directory (`{prefix}/var/brewdock/blobs`).
    #[must_use]
    pub fn blob_dir(&self) -> PathBuf {
        self.var_brewdock().join("blobs")
    }

    /// Extracted bottle store directory (`{prefix}/var/brewdock/store`).
    #[must_use]
    pub fn store_dir(&self) -> PathBuf {
        self.var_brewdock().join("store")
    }

    /// Advisory lock directory (`{prefix}/var/brewdock/locks`).
    #[must_use]
    pub fn lock_dir(&self) -> PathBuf {
        self.var_brewdock().join("locks")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_production_prefix() {
        let layout = Layout::production();
        assert_eq!(layout.prefix(), Path::new("/opt/homebrew"));
    }

    #[test]
    fn test_layout_with_root_prefix() {
        let layout = Layout::with_root("/tmp/test");
        assert_eq!(layout.prefix(), Path::new("/tmp/test/opt/homebrew"));
    }

    #[test]
    fn test_layout_cellar() {
        let layout = Layout::with_root("/tmp/test");
        assert_eq!(
            layout.cellar(),
            PathBuf::from("/tmp/test/opt/homebrew/Cellar")
        );
    }

    #[test]
    fn test_layout_opt_dir() {
        let layout = Layout::with_root("/tmp/test");
        assert_eq!(
            layout.opt_dir(),
            PathBuf::from("/tmp/test/opt/homebrew/opt")
        );
    }

    #[test]
    fn test_layout_bin_dir() {
        let layout = Layout::with_root("/tmp/test");
        assert_eq!(
            layout.bin_dir(),
            PathBuf::from("/tmp/test/opt/homebrew/bin")
        );
    }

    #[test]
    fn test_layout_var_brewdock() {
        let layout = Layout::with_root("/tmp/test");
        assert_eq!(
            layout.var_brewdock(),
            PathBuf::from("/tmp/test/opt/homebrew/var/brewdock")
        );
    }

    #[test]
    fn test_layout_cache_dir() {
        let layout = Layout::with_root("/tmp/test");
        assert_eq!(
            layout.cache_dir(),
            PathBuf::from("/tmp/test/opt/homebrew/var/brewdock/cache")
        );
    }

    #[test]
    fn test_layout_blob_dir() {
        let layout = Layout::with_root("/tmp/test");
        assert_eq!(
            layout.blob_dir(),
            PathBuf::from("/tmp/test/opt/homebrew/var/brewdock/blobs")
        );
    }

    #[test]
    fn test_layout_store_dir() {
        let layout = Layout::with_root("/tmp/test");
        assert_eq!(
            layout.store_dir(),
            PathBuf::from("/tmp/test/opt/homebrew/var/brewdock/store")
        );
    }

    #[test]
    fn test_layout_lock_dir() {
        let layout = Layout::with_root("/tmp/test");
        assert_eq!(
            layout.lock_dir(),
            PathBuf::from("/tmp/test/opt/homebrew/var/brewdock/locks")
        );
    }
}
