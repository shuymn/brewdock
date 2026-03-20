use std::path::Path;

use crate::error::BottleError;

/// Extracts a gzip-compressed tar archive to the destination directory.
///
/// Creates the destination directory and any intermediate directories as needed.
/// The archive's internal directory structure is preserved under `dest`.
///
/// # Errors
///
/// Returns [`BottleError::Io`] if the archive cannot be read or extracted.
pub fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), BottleError> {
    let file = std::fs::File::open(archive)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a tar.gz archive in memory with the given entries.
    fn create_test_tar_gz(
        entries: &[(&str, &[u8])],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use flate2::{Compression, write::GzEncoder};

        let buf = Vec::new();
        let encoder = GzEncoder::new(buf, Compression::default());
        let mut builder = tar::Builder::new(encoder);

        for &(path, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(path)?;
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, contents)?;
        }

        let encoder = builder.into_inner()?;
        let compressed = encoder.finish()?;
        Ok(compressed)
    }

    #[test]
    fn test_extract_tar_gz_creates_files() -> Result<(), Box<dyn std::error::Error>> {
        let archive_data = create_test_tar_gz(&[("hello.txt", b"hello world")])?;

        let dir = tempfile::tempdir()?;
        let archive_path = dir.path().join("test.tar.gz");
        std::fs::write(&archive_path, &archive_data)?;

        let dest = dir.path().join("out");
        extract_tar_gz(&archive_path, &dest)?;

        let content = std::fs::read_to_string(dest.join("hello.txt"))?;
        assert_eq!(content, "hello world");
        Ok(())
    }

    #[test]
    fn test_extract_tar_gz_preserves_tree_structure() -> Result<(), Box<dyn std::error::Error>> {
        let archive_data = create_test_tar_gz(&[
            ("bin/tool", b"#!/bin/sh\necho hi"),
            ("lib/libfoo.dylib", b"fake-dylib"),
            ("share/doc/README", b"readme contents"),
        ])?;

        let dir = tempfile::tempdir()?;
        let archive_path = dir.path().join("bottle.tar.gz");
        std::fs::write(&archive_path, &archive_data)?;

        let dest = dir.path().join("out");
        extract_tar_gz(&archive_path, &dest)?;

        assert_eq!(
            std::fs::read_to_string(dest.join("bin/tool"))?,
            "#!/bin/sh\necho hi"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("lib/libfoo.dylib"))?,
            "fake-dylib"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("share/doc/README"))?,
            "readme contents"
        );
        Ok(())
    }

    #[test]
    fn test_extract_tar_gz_missing_archive_returns_error() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let result = extract_tar_gz(
            &dir.path().join("nonexistent.tar.gz"),
            &dir.path().join("out"),
        );
        assert!(matches!(result, Err(BottleError::Io(_))));
        Ok(())
    }

    #[test]
    fn test_extract_tar_gz_empty_archive() -> Result<(), Box<dyn std::error::Error>> {
        use flate2::{Compression, write::GzEncoder};

        // Create an empty tar.gz (just finalized, no entries).
        let buf = Vec::new();
        let encoder = GzEncoder::new(buf, Compression::default());
        let builder = tar::Builder::new(encoder);
        let encoder = builder.into_inner()?;
        let compressed = encoder.finish()?;

        let dir = tempfile::tempdir()?;
        let archive_path = dir.path().join("empty.tar.gz");
        std::fs::write(&archive_path, &compressed)?;

        let dest = dir.path().join("out");
        extract_tar_gz(&archive_path, &dest)?;
        assert!(dest.exists());
        Ok(())
    }
}
