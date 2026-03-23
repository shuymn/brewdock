use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use brewdock_bottle::extract_tar_gz;

use crate::{BrewdockError, error::SourceBuildError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceArchiveKind {
    TarGz,
}

pub fn source_archive_filename(url: &str) -> Option<&str> {
    let trimmed = url.split('?').next().unwrap_or(url);
    trimmed.rsplit('/').next()
}

pub(super) fn source_archive_kind(url: &str) -> Option<SourceArchiveKind> {
    let filename = source_archive_filename(url)?.to_ascii_lowercase();
    if filename.ends_with(".tar.gz")
        || Path::new(&filename)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("tgz"))
    {
        Some(SourceArchiveKind::TarGz)
    } else {
        None
    }
}

pub fn extract_source_archive(
    archive_path: &Path,
    tempdir_root: &Path,
) -> Result<PathBuf, BrewdockError> {
    let kind = source_archive_kind(
        archive_path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default(),
    )
    .ok_or_else(|| {
        SourceBuildError::UnsupportedSourceArchive(archive_path.display().to_string())
    })?;

    let extract_dir = tempdir_root.join("extract");
    match kind {
        SourceArchiveKind::TarGz => extract_tar_gz(archive_path, &extract_dir)?,
    }
    discover_source_root(&extract_dir)
}

fn discover_source_root(extract_dir: &Path) -> Result<PathBuf, BrewdockError> {
    let mut entries = std::fs::read_dir(extract_dir)?;
    let first = match entries.next() {
        Some(entry) => entry?,
        None => {
            return Err(
                SourceBuildError::MissingSourceRoot(extract_dir.display().to_string()).into(),
            );
        }
    };
    if entries.next().is_none() && first.file_type()?.is_dir() {
        Ok(first.path())
    } else {
        Ok(extract_dir.to_path_buf())
    }
}
