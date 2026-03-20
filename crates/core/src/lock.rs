use std::path::Path;

use fs2::FileExt;

/// Advisory file lock for preventing concurrent brewdock operations.
///
/// The lock is held for the lifetime of this value. Dropping releases the lock.
#[derive(Debug)]
pub struct FileLock {
    _file: std::fs::File,
}

impl FileLock {
    /// Acquires an exclusive advisory lock on the given path.
    ///
    /// Blocks until the lock is available if another process holds it.
    /// Creates the lock file and parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the lock file cannot be created or locked.
    pub fn acquire(path: &Path) -> Result<Self, std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::File::create(path)?;
        file.lock_exclusive()?;
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_acquire_and_release() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let lock_path = dir.path().join("test.lock");

        let lock = FileLock::acquire(&lock_path)?;
        assert!(lock_path.exists());

        // Release via drop.
        drop(lock);

        // Re-acquire should succeed immediately.
        let _lock = FileLock::acquire(&lock_path)?;
        Ok(())
    }

    #[test]
    fn test_lock_creates_parent_directories() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let lock_path = dir.path().join("nested/deep/test.lock");

        let _lock = FileLock::acquire(&lock_path)?;
        assert!(lock_path.exists());
        Ok(())
    }

    #[test]
    fn test_lock_concurrent_blocks_until_released() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let lock_path = dir.path().join("test.lock");

        let lock = FileLock::acquire(&lock_path)?;

        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();

        let handle = std::thread::spawn(move || -> Result<(), std::io::Error> {
            let _ = started_tx.send(());
            let _lock2 = FileLock::acquire(&lock_path)?;
            let _ = acquired_tx.send(());
            Ok(())
        });

        // Wait for thread to start.
        started_rx
            .recv()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        // Allow time for the thread to reach lock_exclusive.
        std::thread::sleep(std::time::Duration::from_millis(50));

        // The thread should still be waiting.
        assert!(acquired_rx.try_recv().is_err());

        // Release lock.
        drop(lock);

        // Thread should now acquire.
        acquired_rx
            .recv()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        handle
            .join()
            .map_err(|_panic_payload| "thread panicked")
            .map_err(std::io::Error::other)??;
        Ok(())
    }
}
