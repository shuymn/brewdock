use std::path::{Component, Path, PathBuf};

use tar::EntryType;

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
    std::fs::create_dir_all(dest)?;
    // Homebrew bottles are built as root. Without this, tar tries to chown
    // every entry, which fails as a non-root user (equivalent to tar's
    // --no-same-owner flag).
    archive.set_preserve_ownerships(false);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_path = normalize_archive_path(&entry.path()?)?;
        validate_entry_target(entry.header().entry_type(), &entry_path, &entry)?;
        if !entry.unpack_in(dest)? {
            return Err(std::io::Error::other(format!(
                "archive entry escaped destination: {}",
                entry_path.display()
            ))
            .into());
        }
    }
    Ok(())
}

fn normalize_archive_path(path: &Path) -> Result<PathBuf, std::io::Error> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(std::io::Error::other(format!(
                        "archive path escapes root: {}",
                        path.display()
                    )));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(std::io::Error::other(format!(
                    "archive path must be relative: {}",
                    path.display()
                )));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(std::io::Error::other("archive path must not be empty"));
    }

    Ok(normalized)
}

fn validate_entry_target<R: std::io::Read>(
    entry_type: EntryType,
    entry_path: &Path,
    entry: &tar::Entry<'_, R>,
) -> Result<(), std::io::Error> {
    if matches!(entry_type, EntryType::Symlink | EntryType::Link) {
        let Some(target) = entry.link_name()? else {
            return Err(std::io::Error::other(format!(
                "archive link missing target: {}",
                entry_path.display()
            )));
        };
        let target = target.into_owned();
        if target.is_absolute() {
            return Err(std::io::Error::other(format!(
                "archive link target must be relative: {} -> {}",
                entry_path.display(),
                target.display()
            )));
        }

        // Relative symlink targets may legitimately traverse outside the
        // archive root in Homebrew bottles, e.g. to `opt/...` or `etc/...`.
        // `tar::Entry::unpack_in` still constrains extraction writes to `dest`,
        // so only hard links need archive-root validation here.
        if entry_type == EntryType::Link {
            let entry_parent = entry_path.parent().unwrap_or_else(|| Path::new(""));
            normalize_archive_path(&entry_parent.join(target))?;
        }
    }

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

    #[test]
    fn test_extract_tar_gz_allows_safe_relative_symlink_targets()
    -> Result<(), Box<dyn std::error::Error>> {
        use flate2::{Compression, write::GzEncoder};

        let buf = Vec::new();
        let encoder = GzEncoder::new(buf, Compression::default());
        let mut builder = tar::Builder::new(encoder);

        let mut file_header = tar::Header::new_gnu();
        file_header.set_path("libexec/tool")?;
        file_header.set_size(4);
        file_header.set_mode(0o644);
        file_header.set_cksum();
        builder.append(&file_header, &b"tool"[..])?;

        let mut link_header = tar::Header::new_gnu();
        link_header.set_path("bin/tool")?;
        link_header.set_entry_type(tar::EntryType::Symlink);
        link_header.set_size(0);
        link_header.set_mode(0o777);
        link_header.set_link_name("../libexec/tool")?;
        link_header.set_cksum();
        builder.append(&link_header, std::io::empty())?;

        let encoder = builder.into_inner()?;
        let archive = encoder.finish()?;

        let dir = tempfile::tempdir()?;
        let archive_path = dir.path().join("safe-symlink.tar.gz");
        std::fs::write(&archive_path, archive)?;

        let dest = dir.path().join("out");
        extract_tar_gz(&archive_path, &dest)?;

        assert_eq!(std::fs::read_to_string(dest.join("bin/tool"))?, "tool");
        Ok(())
    }

    #[test]
    fn test_extract_tar_gz_allows_relative_symlink_to_prefix_path()
    -> Result<(), Box<dyn std::error::Error>> {
        let archive = create_single_link_tar_gz(
            "kafka/4.2.0/libexec/config",
            tar::EntryType::Symlink,
            "../../../../etc/kafka",
        )?;

        let dir = tempfile::tempdir()?;
        let archive_path = dir.path().join("kafka-symlink.tar.gz");
        std::fs::write(&archive_path, archive)?;

        let dest = dir.path().join("out");
        extract_tar_gz(&archive_path, &dest)?;

        let link = dest.join("kafka/4.2.0/libexec/config");
        assert!(link.is_symlink());
        assert_eq!(
            std::fs::read_link(&link)?,
            PathBuf::from("../../../../etc/kafka")
        );
        Ok(())
    }

    #[test]
    fn test_extract_tar_gz_rejects_hardlink_outside_archive_root()
    -> Result<(), Box<dyn std::error::Error>> {
        let archive = create_single_link_tar_gz(
            "demo/1.0/bin/tool",
            tar::EntryType::Link,
            "../../../../etc/passwd",
        )?;

        let dir = tempfile::tempdir()?;
        let archive_path = dir.path().join("bad-hardlink.tar.gz");
        std::fs::write(&archive_path, archive)?;

        let dest = dir.path().join("out");
        let result = extract_tar_gz(&archive_path, &dest);

        assert!(
            result.is_err(),
            "hardlink escaping archive root should fail"
        );
        Ok(())
    }

    fn create_single_link_tar_gz(
        path: &str,
        entry_type: tar::EntryType,
        link_name: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use flate2::{Compression, write::GzEncoder};

        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(encoder);

        let mut header = tar::Header::new_gnu();
        header.set_path(path)?;
        header.set_entry_type(entry_type);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name(link_name)?;
        header.set_cksum();
        builder.append(&header, std::io::empty())?;

        let encoder = builder.into_inner()?;
        Ok(encoder.finish()?)
    }

    fn create_attack_tar_gz() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use flate2::{Compression, write::GzEncoder};

        let buf = Vec::new();
        let encoder = GzEncoder::new(buf, Compression::default());
        let mut builder = tar::Builder::new(encoder);

        let mut header = tar::Header::new_gnu();
        header.set_size(b"owned".len() as u64);
        header.set_mode(0o644);
        let header_bytes = header.as_mut_bytes();
        let path = b"../escape.txt";
        header_bytes[..path.len()].copy_from_slice(path);
        header_bytes[path.len()] = 0;
        header.set_cksum();
        builder.append(&header, &b"owned"[..])?;

        let encoder = builder.into_inner()?;
        Ok(encoder.finish()?)
    }

    #[test]
    fn test_extract_tar_gz_does_not_escape_destination_with_parent_directory_entry()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let archive_path = dir.path().join("attack.tar.gz");
        let dest = dir.path().join("out");
        let victim = dir.path().join("escape.txt");

        std::fs::write(&victim, "original")?;
        std::fs::write(&archive_path, create_attack_tar_gz()?)?;

        let result = extract_tar_gz(&archive_path, &dest);

        assert_eq!(std::fs::read_to_string(&victim)?, "original");
        if let Err(err) = result {
            let _ = err;
        }
        Ok(())
    }
}
