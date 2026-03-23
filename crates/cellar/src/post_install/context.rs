use std::path::{Path, PathBuf};

use super::{
    PathBase, PathCondition, PathExpr, PathSegment, PlatformContext, PostInstallContext,
    SegmentPart,
};
use crate::{error::CellarError, fs::normalize_absolute_path};

impl PostInstallContext {
    /// Creates a new context for a materialized keg.
    #[must_use]
    pub fn new(
        prefix: &Path,
        keg_path: &Path,
        formula_version: &str,
        platform: &PlatformContext,
    ) -> Self {
        Self {
            formula_name: formula_name_from_keg(keg_path),
            formula_version: formula_version.to_owned(),
            prefix: prefix.to_path_buf(),
            keg_path: keg_path.to_path_buf(),
            kernel_version_major: platform.kernel_version_major.clone(),
            macos_version: platform.macos_version.clone(),
            cpu_arch: platform.cpu_arch.clone(),
            env_overrides: std::collections::BTreeMap::new(),
            captured_outputs: std::collections::BTreeMap::new(),
        }
    }

    pub(super) fn resolve_allowed_path(&self, expr: &PathExpr) -> Result<PathBuf, CellarError> {
        let raw = self.resolve_path(expr);
        let resolved = normalize_absolute_path(&raw).ok_or_else(|| {
            CellarError::UnsupportedPostInstallSyntax {
                message: format!("path escapes allowed roots: {}", raw.display()),
            }
        })?;
        if path_is_allowed(&resolved, self) {
            return Ok(resolved);
        }

        Err(CellarError::UnsupportedPostInstallSyntax {
            message: format!("path escapes allowed roots: {}", resolved.display()),
        })
    }

    fn resolve_path(&self, expr: &PathExpr) -> PathBuf {
        let mut path = match expr.base {
            PathBase::Prefix => self.keg_path.clone(),
            PathBase::Bin => self.keg_path.join("bin"),
            PathBase::Etc => self.prefix.join("etc"),
            PathBase::FormulaPkgetc(ref formula) => self.prefix.join("etc").join(formula),
            PathBase::FormulaOptBin(ref formula) => {
                self.prefix.join("opt").join(formula).join("bin")
            }
            PathBase::HomebrewPrefix => self.prefix.clone(),
            PathBase::Lib => self.keg_path.join("lib"),
            PathBase::Libexec => self.keg_path.join("libexec"),
            PathBase::Pkgetc => self.prefix.join("etc").join(&self.formula_name),
            PathBase::Pkgshare => self.keg_path.join("share").join(&self.formula_name),
            PathBase::Share => self.keg_path.join("share"),
            PathBase::Sbin => self.keg_path.join("sbin"),
            PathBase::Var => self.prefix.join("var"),
        };
        for segment in &expr.segments {
            path.push(self.resolve_segment(segment));
        }
        path
    }

    fn resolve_segment(&self, segment: &PathSegment) -> String {
        match segment {
            PathSegment::Literal(s) => s.clone(),
            PathSegment::Interpolated(parts) => {
                let mut result = String::new();
                for part in parts {
                    result.push_str(&self.resolve_segment_part(part));
                }
                result
            }
        }
    }

    pub(super) fn resolve_segment_part(&self, part: &SegmentPart) -> String {
        match part {
            SegmentPart::Literal(s) => s.clone(),
            SegmentPart::FormulaName => self.formula_name.clone(),
            SegmentPart::VersionMajorMinor => major_minor_version(&self.formula_version),
            SegmentPart::VersionMajor => major_version(&self.formula_version),
            SegmentPart::KernelVersionMajor => self.kernel_version_major.clone(),
            SegmentPart::MacOSVersion => self.macos_version.clone(),
            SegmentPart::CpuArch => self.cpu_arch.clone(),
            SegmentPart::CapturedOutput(name) => {
                self.captured_outputs.get(name).cloned().unwrap_or_default()
            }
            SegmentPart::CapturedOutputBasename(name) => self
                .captured_outputs
                .get(name)
                .and_then(|value| Path::new(value).file_name())
                .and_then(|name| name.to_str())
                .map_or_else(String::new, ToOwned::to_owned),
        }
    }
}

/// Extracts `"major.minor"` from a version string like `"3.12.2"`.
fn major_minor_version(version: &str) -> String {
    let mut parts = version.splitn(3, '.');
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) => format!("{major}.{minor}"),
        _ => version.to_owned(),
    }
}

/// Extracts the major component from a version string like `"17.2"` → `"17"`.
fn major_version(version: &str) -> String {
    version.split('.').next().unwrap_or(version).to_owned()
}

fn formula_name_from_keg(keg_path: &Path) -> String {
    keg_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map_or_else(String::new, ToOwned::to_owned)
}

pub(super) fn path_condition_matches(
    context: &PostInstallContext,
    condition: &PathExpr,
    kind: PathCondition,
) -> Result<bool, CellarError> {
    let path = context.resolve_allowed_path(condition)?;
    Ok(match kind {
        PathCondition::Exists => path.exists(),
        PathCondition::Missing => !path.exists(),
        PathCondition::Symlink => path.is_symlink(),
        PathCondition::ExistsAndNotSymlink => path.exists() && !path.is_symlink(),
    })
}

const ALLOWED_PREFIX_DIRS: &[&str] = &[
    "etc", "var", "share", "bin", "sbin", "lib", "include", "opt",
];

fn path_is_allowed(path: &Path, context: &PostInstallContext) -> bool {
    path.starts_with(&context.keg_path)
        || path
            .strip_prefix(&context.prefix)
            .ok()
            .and_then(|rel| rel.components().next())
            .and_then(|c| c.as_os_str().to_str())
            .is_some_and(|first| ALLOWED_PREFIX_DIRS.contains(&first))
}

pub(super) fn install_symlink_path(link_dir: &Path, target: &Path) -> Result<PathBuf, CellarError> {
    let Some(name) = target.file_name() else {
        return Err(CellarError::UnsupportedPostInstallSyntax {
            message: "install_symlink target must have file name".to_owned(),
        });
    };
    Ok(link_dir.join(name))
}
