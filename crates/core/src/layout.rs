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
    fn test_layout_prefixes() {
        assert_eq!(Layout::production().prefix(), Path::new("/opt/homebrew"));
        assert_eq!(
            Layout::with_root("/tmp/test").prefix(),
            Path::new("/tmp/test/opt/homebrew")
        );
    }

    #[test]
    fn test_layout_derived_paths() {
        let layout = Layout::with_root("/tmp/test");
        let expected_paths = [
            layout.cellar(),
            layout.opt_dir(),
            layout.bin_dir(),
            layout.var_brewdock(),
            layout.cache_dir(),
            layout.blob_dir(),
            layout.store_dir(),
            layout.lock_dir(),
        ];
        let expected_suffixes = [
            "Cellar",
            "opt",
            "bin",
            "var/brewdock",
            "var/brewdock/cache",
            "var/brewdock/blobs",
            "var/brewdock/store",
            "var/brewdock/locks",
        ];

        for (actual, suffix) in expected_paths.iter().zip(expected_suffixes) {
            assert_eq!(
                actual,
                &PathBuf::from("/tmp/test/opt/homebrew").join(suffix),
                "path mismatch for suffix {suffix}"
            );
        }
    }
}
