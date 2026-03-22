use std::{
    collections::BTreeSet,
    ffi::OsString,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

pub use brewdock_analysis::{
    Argument, ContentPart, PathBase, PathCondition, PathExpr, PathSegment, Program, SegmentPart,
    Statement, extract_post_install_block, lower_post_install, lower_post_install_tier2,
    validate_post_install,
};

use crate::{error::CellarError, fs::normalize_absolute_path, link::relative_from_to};

static ROLLBACK_NONCE: AtomicUsize = AtomicUsize::new(0);

/// Execution environment for the restricted `post_install` DSL.
#[derive(Debug, Clone)]
pub struct PostInstallContext {
    formula_name: String,
    formula_version: String,
    prefix: PathBuf,
    keg_path: PathBuf,
    kernel_version_major: String,
    macos_version: String,
    cpu_arch: String,
    /// Environment variables set during `post_install` and applied to spawned commands.
    env_overrides: std::collections::BTreeMap<String, String>,
    /// Captured process outputs (from [`Statement::ProcessCapture`]).
    captured_outputs: std::collections::BTreeMap<String, String>,
}

/// Rollback handle for a completed `post_install` execution.
#[derive(Debug)]
pub struct PostInstallTransaction {
    backups: Vec<(PathBuf, Option<PathBuf>)>,
    rollback_dir: PathBuf,
}

/// Runtime platform information for Tier 2 DSL evaluation.
#[derive(Debug, Clone)]
pub struct PlatformContext {
    /// OS kernel version major (e.g. `"24"` on macOS Sequoia).
    pub kernel_version_major: String,
    /// macOS version string (e.g. `"15.1"`).
    pub macos_version: String,
    /// CPU architecture (e.g. `"arm64"`).
    pub cpu_arch: String,
}

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

    fn resolve_segment_part(&self, part: &SegmentPart) -> String {
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
        }
    }

    fn resolve_allowed_path(&self, expr: &PathExpr) -> Result<PathBuf, CellarError> {
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

/// # Errors
///
/// Returns [`CellarError::Analysis`] for any unsupported Ruby construct and
/// [`CellarError::PostInstallCommandFailed`] when a spawned command exits
/// unsuccessfully.
pub fn run_post_install(
    source: &str,
    context: &mut PostInstallContext,
) -> Result<PostInstallTransaction, CellarError> {
    // Tier 1 → Tier 2 fallback: try static analysis first, then attribute injection.
    let program = lower_post_install(source, &context.formula_version)
        .or_else(|_| lower_post_install_tier2(source, &context.formula_version))?;
    let rollback_roots = collect_rollback_roots(&program, context);
    run_with_rollback(&rollback_roots, context, |ctx| {
        execute_statements(&program.statements, ctx)
    })
}

impl PostInstallTransaction {
    /// Commits a successful `post_install` execution and drops rollback data.
    ///
    /// # Errors
    ///
    /// Returns [`CellarError::Io`] if cleanup of rollback metadata fails.
    pub fn commit(self) -> Result<(), CellarError> {
        std::fs::remove_dir_all(self.rollback_dir)?;
        Ok(())
    }

    /// Restores the pre-hook filesystem state.
    ///
    /// # Errors
    ///
    /// Returns [`CellarError::Io`] if restore fails.
    pub fn rollback(self) -> Result<(), CellarError> {
        restore_backups(&self.backups)?;
        std::fs::remove_dir_all(self.rollback_dir)?;
        Ok(())
    }
}

fn collect_rollback_roots(program: &Program, context: &PostInstallContext) -> Vec<PathBuf> {
    let mut roots = BTreeSet::new();
    collect_statement_roots(&program.statements, context, &mut roots);
    collapse_nested_roots(roots.into_iter().collect())
}

fn collect_statement_roots(
    statements: &[Statement],
    context: &PostInstallContext,
    roots: &mut BTreeSet<PathBuf>,
) {
    for statement in statements {
        match statement {
            Statement::Copy { to, .. }
            | Statement::RecursiveCopy { to, .. }
            | Statement::CopyChildren { to_dir: to, .. }
            | Statement::WriteFile { path: to, .. }
            | Statement::GlobRemove { dir: to, .. }
            | Statement::GlobSymlink { link_dir: to, .. }
            | Statement::ForceSymlink { link: to, .. }
            | Statement::Install { into_dir: to, .. }
            | Statement::Chmod { path: to, .. } => {
                if let Ok(path) = context.resolve_allowed_path(to)
                    && let Some(root) = rollback_root(&path, context)
                {
                    roots.insert(root);
                }
            }
            Statement::Mkpath(path) | Statement::RemoveIfExists(path) => {
                if let Ok(path) = context.resolve_allowed_path(path)
                    && let Some(root) = rollback_root(&path, context)
                {
                    roots.insert(root);
                }
            }
            Statement::InstallSymlink { link_dir, target } => {
                if let Ok(link_dir) = context.resolve_allowed_path(link_dir)
                    && let Ok(target) = context.resolve_allowed_path(target)
                    && let Ok(link_path) = install_symlink_path(&link_dir, &target)
                    && let Some(root) = rollback_root(&link_path, context)
                {
                    roots.insert(root);
                }
            }
            Statement::System(arguments) => {
                for argument in arguments.iter().skip(1) {
                    if let Argument::Path(path) = argument
                        && let Ok(path) = context.resolve_allowed_path(path)
                        && let Some(root) = rollback_root(&path, context)
                    {
                        roots.insert(root);
                    }
                }
            }
            Statement::IfPath { then_branch, .. } | Statement::IfEnv { then_branch, .. } => {
                collect_statement_roots(then_branch, context, roots);
            }
            Statement::MirrorTree { dest, .. } => {
                if let Ok(path) = context.resolve_allowed_path(dest)
                    && let Some(root) = rollback_root(&path, context)
                {
                    roots.insert(root);
                }
            }
            Statement::ChildrenSymlink { link_dir, .. } => {
                if let Ok(path) = context.resolve_allowed_path(link_dir)
                    && let Some(root) = rollback_root(&path, context)
                {
                    roots.insert(root);
                }
            }
            Statement::ProcessCapture { .. } | Statement::SetEnv { .. } => {
                // No filesystem mutation — nothing to rollback.
            }
        }
    }
}

fn rollback_root(path: &Path, context: &PostInstallContext) -> Option<PathBuf> {
    if path.starts_with(&context.keg_path) || !path.starts_with(&context.prefix) {
        return None;
    }

    let relative = path.strip_prefix(&context.prefix).ok()?;
    let mut components = relative.components();
    let first = PathBuf::from(components.next()?.as_os_str());
    let Some(second) = components.next() else {
        return Some(context.prefix.join(first));
    };

    if first == Path::new("etc") || first == Path::new("var") || first == Path::new("share") {
        return Some(context.prefix.join(first).join(second.as_os_str()));
    }

    Some(path.to_path_buf())
}

fn collapse_nested_roots(mut roots: Vec<PathBuf>) -> Vec<PathBuf> {
    roots.sort();
    let mut collapsed = Vec::new();
    for root in roots {
        if collapsed
            .iter()
            .any(|existing: &PathBuf| root.starts_with(existing))
        {
            continue;
        }
        collapsed.push(root);
    }
    collapsed
}

fn run_with_rollback<F>(
    rollback_roots: &[PathBuf],
    context: &mut PostInstallContext,
    run: F,
) -> Result<PostInstallTransaction, CellarError>
where
    F: FnOnce(&mut PostInstallContext) -> Result<(), CellarError>,
{
    let rollback_dir = make_rollback_dir()?;
    let backups = rollback_roots
        .iter()
        .map(|root| {
            let backup = if root.symlink_metadata().is_ok() {
                let backup = rollback_dir.join(format!("entry-{}", backups_len_hint()));
                copy_path(root, &backup)?;
                Some(backup)
            } else {
                None
            };
            Ok((root.clone(), backup))
        })
        .collect::<Result<Vec<_>, CellarError>>()?;

    match run(context) {
        Ok(()) => Ok(PostInstallTransaction {
            backups,
            rollback_dir,
        }),
        Err(error) => {
            restore_backups(&backups)?;
            std::fs::remove_dir_all(&rollback_dir)?;
            Err(error)
        }
    }
}

fn backups_len_hint() -> usize {
    ROLLBACK_NONCE.fetch_add(1, Ordering::Relaxed)
}

fn make_rollback_dir() -> Result<PathBuf, CellarError> {
    let dir = std::env::temp_dir().join(format!(
        "brewdock-post-install-{}-{}",
        std::process::id(),
        ROLLBACK_NONCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn restore_backups(backups: &[(PathBuf, Option<PathBuf>)]) -> Result<(), CellarError> {
    for (root, backup) in backups {
        remove_path_if_exists(root)?;
        if let Some(backup) = backup {
            copy_path(backup, root)?;
        }
    }
    Ok(())
}

fn remove_path_if_exists(path: &Path) -> Result<(), CellarError> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_file() => {
            std::fs::remove_file(path)?;
            Ok(())
        }
        Ok(metadata) if metadata.is_dir() => {
            std::fs::remove_dir_all(path)?;
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn copy_path(from: &Path, to: &Path) -> Result<(), CellarError> {
    let metadata = from.symlink_metadata()?;
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(from)?;
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::os::unix::fs::symlink(target, to)?;
        return Ok(());
    }

    if metadata.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_path(&entry.path(), &to.join(entry.file_name()))?;
        }
        return Ok(());
    }

    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to)?;
    Ok(())
}

fn copy_children(from: &Path, to: &Path) -> Result<(), CellarError> {
    std::fs::create_dir_all(to)?;
    if !from.is_dir() {
        return Ok(());
    }

    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        copy_path(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

fn path_condition_matches(
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
        || ALLOWED_PREFIX_DIRS
            .iter()
            .any(|&dir| path.starts_with(context.prefix.join(dir)))
}

fn execute_statements(
    statements: &[Statement],
    context: &mut PostInstallContext,
) -> Result<(), CellarError> {
    for statement in statements {
        match statement {
            Statement::Mkpath(path) => {
                std::fs::create_dir_all(context.resolve_allowed_path(path)?)?;
            }
            Statement::Copy { from, to } => {
                let from = context.resolve_allowed_path(from)?;
                let to = context.resolve_allowed_path(to)?;
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(from, to)?;
            }
            Statement::RemoveIfExists(path) => {
                remove_path_if_exists(&context.resolve_allowed_path(path)?)?;
            }
            Statement::InstallSymlink { link_dir, target } => {
                let link_dir = context.resolve_allowed_path(link_dir)?;
                std::fs::create_dir_all(&link_dir)?;
                let target = context.resolve_allowed_path(target)?;
                let link_path = install_symlink_path(&link_dir, &target)?;
                remove_path_if_exists(&link_path)?;
                let link_target = relative_from_to(&link_dir, &target);
                std::os::unix::fs::symlink(link_target, link_path)?;
            }
            Statement::System(arguments) => run_system(arguments, context)?,
            Statement::IfPath {
                condition,
                kind,
                then_branch,
            } => {
                if path_condition_matches(context, condition, *kind)? {
                    execute_statements(then_branch, context)?;
                }
            }
            Statement::RecursiveCopy { from, to } => {
                let from_path = context.resolve_allowed_path(from)?;
                let to_path = context.resolve_allowed_path(to)?;
                let file_name =
                    from_path
                        .file_name()
                        .ok_or_else(|| CellarError::InvalidPathComponent {
                            path: from_path.clone(),
                        })?;
                let dest = to_path.join(file_name);
                copy_path(&from_path, &dest)?;
            }
            Statement::CopyChildren { from_dir, to_dir } => {
                let from_path = context.resolve_allowed_path(from_dir)?;
                let to_path = context.resolve_allowed_path(to_dir)?;
                copy_children(&from_path, &to_path)?;
            }
            Statement::ForceSymlink { target, link } => {
                let target_path = context.resolve_allowed_path(target)?;
                let link_path = context.resolve_allowed_path(link)?;
                let link_parent =
                    link_path
                        .parent()
                        .ok_or_else(|| CellarError::MissingParentDirectory {
                            path: link_path.clone(),
                        })?;
                std::fs::create_dir_all(link_parent)?;
                remove_path_if_exists(&link_path)?;
                let rel_target = relative_from_to(link_parent, &target_path);
                std::os::unix::fs::symlink(rel_target, &link_path)?;
            }
            Statement::WriteFile { path, content } => {
                let file_path = context.resolve_allowed_path(path)?;
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let resolved = resolve_content(content, context);
                std::fs::write(file_path, resolved)?;
            }
            Statement::GlobRemove { dir, pattern } => {
                let dir_path = context.resolve_allowed_path(dir)?;
                if dir_path.is_dir() {
                    for entry in std::fs::read_dir(&dir_path)? {
                        let entry = entry?;
                        if entry
                            .file_name()
                            .to_str()
                            .is_some_and(|name| glob_matches(name, pattern))
                        {
                            remove_path_if_exists(&entry.path())?;
                        }
                    }
                }
            }
            Statement::GlobSymlink {
                source_dir,
                pattern,
                link_dir,
            } => {
                let source_path = context.resolve_allowed_path(source_dir)?;
                let link_path = context.resolve_allowed_path(link_dir)?;
                std::fs::create_dir_all(&link_path)?;
                if source_path.is_dir() {
                    for entry in std::fs::read_dir(&source_path)? {
                        let entry = entry?;
                        if entry
                            .file_name()
                            .to_str()
                            .is_some_and(|name| glob_matches(name, pattern))
                        {
                            let name = entry.file_name();
                            let link = link_path.join(&name);
                            remove_path_if_exists(&link)?;
                            let rel = relative_from_to(&link_path, &entry.path());
                            std::os::unix::fs::symlink(rel, &link)?;
                        }
                    }
                }
            }
            Statement::Install { into_dir, from } => {
                let from_path = context.resolve_allowed_path(from)?;
                let into_path = context.resolve_allowed_path(into_dir)?;
                std::fs::create_dir_all(&into_path)?;
                let file_name =
                    from_path
                        .file_name()
                        .ok_or_else(|| CellarError::InvalidPathComponent {
                            path: from_path.clone(),
                        })?;
                let dest = into_path.join(file_name);
                // Homebrew's install uses FileUtils.mv (move semantics).
                std::fs::rename(&from_path, &dest).or_else(|_rename_err| {
                    std::fs::copy(&from_path, &dest)?;
                    std::fs::remove_file(&from_path)
                })?;
            }
            Statement::Chmod { path, mode } => {
                let file_path = context.resolve_allowed_path(path)?;
                let permissions = std::fs::Permissions::from_mode(*mode);
                std::fs::set_permissions(file_path, permissions)?;
            }
            Statement::MirrorTree {
                source,
                dest,
                prune_names,
            } => {
                execute_mirror_tree(
                    &context.resolve_allowed_path(source)?,
                    &context.resolve_allowed_path(dest)?,
                    prune_names,
                )?;
            }
            Statement::ChildrenSymlink {
                source_dir,
                link_dir,
                suffix,
            } => {
                let source_path = context.resolve_allowed_path(source_dir)?;
                let link_path = context.resolve_allowed_path(link_dir)?;
                std::fs::create_dir_all(&link_path)?;
                if source_path.is_dir() {
                    for entry in std::fs::read_dir(&source_path)? {
                        let entry = entry?;
                        let basename = entry.file_name();
                        let basename_str = basename.to_string_lossy();
                        let mut link_name = basename_str.to_string();
                        for part in suffix {
                            link_name.push_str(&context.resolve_segment_part(part));
                        }
                        let link = link_path.join(&link_name);
                        remove_path_if_exists(&link)?;
                        let rel = relative_from_to(&link_path, &entry.path());
                        std::os::unix::fs::symlink(rel, &link)?;
                    }
                }
            }
            Statement::IfEnv {
                variable,
                negate,
                then_branch,
            } => {
                let is_set = std::env::var(variable).is_ok_and(|v| !v.is_empty());
                let should_execute = if *negate { !is_set } else { is_set };
                if should_execute {
                    execute_statements(then_branch, context)?;
                }
            }
            Statement::SetEnv { variable, value } => {
                let resolved = resolve_content(value, context);
                context.env_overrides.insert(variable.clone(), resolved);
            }
            Statement::ProcessCapture { variable, command } => {
                let output = run_process_capture(command, context)?;
                context.captured_outputs.insert(variable.clone(), output);
            }
        }
    }
    Ok(())
}

/// Mirrors a source directory tree into a destination directory using symlinks.
///
/// This implements the Homebrew `Pathname#find` + `relative_path_from` + `install_symlink`
/// pattern used by postgresql and similar formulas to create versioned include/lib/share trees.
fn execute_mirror_tree(
    source: &Path,
    dest: &Path,
    prune_names: &[String],
) -> Result<(), CellarError> {
    if !source.is_dir() {
        return Ok(());
    }
    mirror_tree_walk(source, source, dest, prune_names)
}

fn mirror_tree_walk(
    root: &Path,
    current: &Path,
    dest_root: &Path,
    prune_names: &[String],
) -> Result<(), CellarError> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Prune matching names
        if prune_names.iter().any(|p| p == name_str.as_ref()) {
            continue;
        }

        let src = entry.path();
        let relative = src
            .strip_prefix(root)
            .map_err(|err| CellarError::Io(std::io::Error::other(err)))?;
        let dst = dest_root.join(relative);

        let metadata = entry.metadata()?;
        if metadata.is_dir() && !entry.file_type()?.is_symlink() {
            // Retain existing real directories at destination
            if !dst.is_dir() || dst.is_symlink() {
                // Remove conflicting entry and create directory
                remove_path_if_exists(&dst)?;
                std::fs::create_dir_all(&dst)?;
            }
            mirror_tree_walk(root, &src, dest_root, prune_names)?;
        } else {
            // File or symlink: create a symlink in dest
            remove_path_if_exists(&dst)?;
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
                let rel_target = relative_from_to(parent, &src);
                std::os::unix::fs::symlink(rel_target, &dst)?;
            }
        }
    }
    Ok(())
}

/// Resolves DSL arguments into OS strings and splits into program + args.
///
/// Returns `(program, args)` where `program` is the first argument.
fn resolve_command_line(
    arguments: &[Argument],
    context: &PostInstallContext,
    caller: &str,
) -> Result<(OsString, Vec<OsString>), CellarError> {
    let mut command_line = arguments
        .iter()
        .map(|arg| match arg {
            Argument::Path(path) => Ok(context.resolve_allowed_path(path)?.into_os_string()),
            Argument::String(value) => Ok(OsString::from(value)),
        })
        .collect::<Result<Vec<_>, CellarError>>()?;

    if command_line.is_empty() {
        return Err(CellarError::UnsupportedPostInstallSyntax {
            message: format!("{caller} expects at least one argument"),
        });
    }

    let program = command_line.remove(0);
    Ok((program, command_line))
}

/// Builds a [`CellarError::PostInstallCommandFailed`] from process output.
fn command_failed_error(output: &std::process::Output) -> CellarError {
    let stderr = String::from_utf8_lossy(&output.stderr);
    CellarError::PostInstallCommandFailed {
        message: if stderr.trim().is_empty() {
            format!("exit status {}", output.status)
        } else {
            stderr.trim().to_owned()
        },
    }
}

fn run_process_capture(
    command: &[Argument],
    context: &PostInstallContext,
) -> Result<String, CellarError> {
    let output = command_output(command, context, "process capture")?;
    if !output.status.success() {
        return Err(command_failed_error(&output));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok(stdout)
}

fn install_symlink_path(link_dir: &Path, target: &Path) -> Result<PathBuf, CellarError> {
    let Some(name) = target.file_name() else {
        return Err(CellarError::UnsupportedPostInstallSyntax {
            message: "install_symlink target must have file name".to_owned(),
        });
    };
    Ok(link_dir.join(name))
}

fn run_system(arguments: &[Argument], context: &PostInstallContext) -> Result<(), CellarError> {
    let (program, program_args) = resolve_command_line(arguments, context, "system")?;

    let span = tracing::info_span!(
        "bd.child_process",
        program = %program.to_string_lossy(),
        argv = %program_args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let _entered = span.enter();
    let output = Command::new(&program)
        .args(&program_args)
        .envs(&context.env_overrides)
        .output()?;
    if output.status.success() {
        return Ok(());
    }

    Err(command_failed_error(&output))
}

fn command_output(
    arguments: &[Argument],
    context: &PostInstallContext,
    caller: &str,
) -> Result<std::process::Output, CellarError> {
    let (program, program_args) = resolve_command_line(arguments, context, caller)?;
    Command::new(&program)
        .args(&program_args)
        .envs(&context.env_overrides)
        .output()
        .map_err(CellarError::from)
}

fn glob_matches(name: &str, pattern: &str) -> bool {
    let Some(prefix) = pattern.strip_suffix('*') else {
        return name == pattern;
    };
    prefix
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .map_or_else(
            || name.starts_with(prefix),
            |alternatives| alternatives.split(',').any(|alt| name.starts_with(alt)),
        )
}

fn resolve_content(parts: &[ContentPart], context: &PostInstallContext) -> String {
    let mut result = String::new();
    for part in parts {
        match part {
            ContentPart::Literal(s) => result.push_str(s),
            ContentPart::HomebrewPrefix => {
                result.push_str(&context.prefix.to_string_lossy());
            }
            ContentPart::Runtime(segment_part) => {
                result.push_str(&context.resolve_segment_part(segment_part));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_platform() -> PlatformContext {
        PlatformContext {
            kernel_version_major: "24".to_owned(),
            macos_version: "15.1".to_owned(),
            cpu_arch: "arm64".to_owned(),
        }
    }

    fn test_context(prefix: &Path, keg: &Path, version: &str) -> PostInstallContext {
        PostInstallContext::new(prefix, keg, version, &test_platform())
    }

    fn write_executable(path: &Path, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
        std::fs::write(path, contents)?;
        let mut perms = std::fs::metadata(path)?.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
        }
        std::fs::set_permissions(path, perms)?;
        Ok(())
    }

    fn shared_mime_info_post_install_source() -> &'static str {
        r#"
class SharedMimeInfo < Formula
  def post_install
    global_mime = HOMEBREW_PREFIX/"share/mime"
    cellar_mime = share/"mime"

    rm_r(global_mime) if global_mime.symlink?
    rm_r(cellar_mime) if cellar_mime.exist? && !cellar_mime.symlink?
    ln_sf(global_mime, cellar_mime)

    (global_mime/"packages").mkpath
    cp (pkgshare/"packages").children, global_mime/"packages"

    system bin/"update-mime-database", global_mime
  end
end
"#
    }

    #[test]
    fn test_extract_post_install_block() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  def post_install
    (var/"demo").mkpath
    if (prefix/"flag").exist?
      cp share/"src.txt", var/"demo/dst.txt"
    end
  end
end
"#;

        let block = extract_post_install_block(source)?;

        assert!(block.contains(r#"(var/"demo").mkpath"#));
        assert!(block.contains(r#"cp share/"src.txt", var/"demo/dst.txt""#));
        Ok(())
    }

    #[test]
    fn test_run_post_install_executes_supported_subset() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/demo/1.0");
        std::fs::create_dir_all(keg.join("share"))?;
        std::fs::create_dir_all(keg.join("bin"))?;
        std::fs::write(keg.join("share/src.txt"), "payload")?;
        std::fs::write(
            keg.join("bin/write-flag"),
            "#!/bin/sh\nprintf '%s' \"$1\" > \"$2\"\n",
        )?;
        let mut perms = std::fs::metadata(keg.join("bin/write-flag"))?.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
        }
        std::fs::set_permissions(keg.join("bin/write-flag"), perms)?;
        std::fs::write(keg.join("flag"), "go")?;

        let source = r#"
class Demo < Formula
  def post_install
    (var/"demo").mkpath
    cp share/"src.txt", var/"demo/copied.txt"
    if (prefix/"flag").exist?
      system bin/"write-flag", "done", var/"demo/result.txt"
    end
  end
end
"#;

        run_post_install(source, &mut test_context(&prefix, &keg, "1.0"))?.commit()?;

        assert_eq!(
            std::fs::read_to_string(prefix.join("var/demo/copied.txt"))?,
            "payload"
        );
        assert_eq!(
            std::fs::read_to_string(prefix.join("var/demo/result.txt"))?,
            "done"
        );
        Ok(())
    }

    #[test]
    fn test_run_post_install_rejects_empty_source() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;

        let result = run_post_install("", &mut test_context(&prefix, &keg, "1.0"));
        assert!(matches!(result, Err(CellarError::Analysis(_))));
        Ok(())
    }

    #[test]
    fn test_run_post_install_rejects_unsupported_syntax() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;
        let source = r#"
class Demo < Formula
  def post_install
    puts "nope"
  end
end
"#;

        let result = run_post_install(source, &mut test_context(&prefix, &keg, "1.0"));
        assert!(matches!(result, Err(CellarError::Analysis(_))));
        Ok(())
    }

    #[test]
    fn test_run_post_install_bootstraps_certificate_bundle_pattern_on_macos()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/ca-certificates/1.0");
        std::fs::create_dir_all(keg.join("share/ca-certificates"))?;
        std::fs::write(
            keg.join("share/ca-certificates/cacert.pem"),
            "mozilla-bundle",
        )?;

        let source = r#"
class CaCertificates < Formula
  def post_install
    if OS.mac?
      macos_post_install
    else
      linux_post_install
    end
  end

  def macos_post_install
    pkgetc.mkpath
    (pkgetc/"cert.pem").atomic_write("ignored")
  end

  def linux_post_install
    cp pkgshare/"cacert.pem", pkgetc/"cert.pem"
  end
end
"#;

        run_post_install(source, &mut test_context(&prefix, &keg, "1.0"))?.commit()?;

        assert_eq!(
            std::fs::read_to_string(prefix.join("etc/ca-certificates/cert.pem"))?,
            "mozilla-bundle"
        );
        Ok(())
    }

    #[test]
    fn test_run_post_install_bootstraps_certificate_bundle_via_runtime_helper_resolution()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/ca-certificates/1.0");
        std::fs::create_dir_all(keg.join("share/ca-certificates"))?;
        std::fs::write(
            keg.join("share/ca-certificates/cacert.pem"),
            "mozilla-bundle",
        )?;

        let source = r#"
class CaCertificates < Formula
  def post_install
    if OS.mac?
      bootstrap_bundle
    else
      unsupported_linux_path
    end
  end

  def bootstrap_bundle
    pkgetc.mkpath
    (pkgetc/"cert.pem").atomic_write("ignored")
  end

  def unsupported_linux_path
    puts "linux only"
  end
end
"#;

        run_post_install(source, &mut test_context(&prefix, &keg, "1.0"))?.commit()?;

        assert_eq!(
            std::fs::read_to_string(prefix.join("etc/ca-certificates/cert.pem"))?,
            "mozilla-bundle"
        );
        Ok(())
    }

    #[test]
    fn test_run_post_install_bootstraps_openssl_cert_symlink_pattern()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/openssl@3/1.0");
        std::fs::create_dir_all(prefix.join("etc/ca-certificates"))?;
        std::fs::write(prefix.join("etc/ca-certificates/cert.pem"), "bundle")?;

        let source = r#"
class OpensslAT3 < Formula
  def openssldir
    etc/"openssl@3"
  end

  def post_install
    rm(openssldir/"cert.pem") if (openssldir/"cert.pem").exist?
    openssldir.install_symlink Formula["ca-certificates"].pkgetc/"cert.pem"
  end
end
"#;

        run_post_install(source, &mut test_context(&prefix, &keg, "1.0"))?.commit()?;

        let cert_link = prefix.join("etc/openssl@3/cert.pem");
        assert!(cert_link.is_symlink());
        assert_eq!(std::fs::read_to_string(cert_link)?, "bundle");
        Ok(())
    }

    #[test]
    fn test_run_post_install_bootstraps_cert_symlink_via_path_helper_resolution()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/openssl@3/1.0");
        std::fs::create_dir_all(prefix.join("etc/ca-certificates"))?;
        std::fs::write(prefix.join("etc/ca-certificates/cert.pem"), "bundle")?;

        let source = r#"
class OpensslAT3 < Formula
  def cert_dir
    etc/"openssl@3"
  end

  def post_install
    rm(cert_dir/"cert.pem") if (cert_dir/"cert.pem").exist?
    cert_dir.install_symlink Formula["ca-certificates"].pkgetc/"cert.pem"
  end
end
"#;

        run_post_install(source, &mut test_context(&prefix, &keg, "1.0"))?.commit()?;

        let cert_link = prefix.join("etc/openssl@3/cert.pem");
        assert!(cert_link.is_symlink());
        assert_eq!(std::fs::read_to_string(cert_link)?, "bundle");
        Ok(())
    }

    #[test]
    fn test_run_post_install_rolls_back_prefix_mutation_on_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/demo/1.0");
        std::fs::create_dir_all(keg.join("share"))?;
        std::fs::write(keg.join("share/src.txt"), "payload")?;

        let source = r#"
class Demo < Formula
  def post_install
    (var/"demo").mkpath
    cp share/"src.txt", var/"demo/copied.txt"
    system "/bin/sh", "-c", "exit 1"
  end
end
"#;

        let result = run_post_install(source, &mut test_context(&prefix, &keg, "1.0"));

        assert!(matches!(
            result,
            Err(CellarError::PostInstallCommandFailed { .. })
        ));
        assert!(!prefix.join("var/demo").exists());
        Ok(())
    }

    #[test]
    fn test_run_post_install_rejects_path_traversal_and_leaves_no_escape_artifacts()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;

        let escape = prefix.join("escape");
        let source = r#"
class Demo < Formula
  def post_install
    (var/"demo"/".."/".."/"escape").mkpath
    system "/bin/sh", "-c", "exit 1"
  end
end
"#;

        let result = run_post_install(source, &mut test_context(&prefix, &keg, "1.0"));
        assert!(
            result.is_err(),
            "path traversal in post_install should fail closed before mutating outside the prefix"
        );
        assert!(
            !escape.exists(),
            "path traversal should not leave artifacts outside the prefix"
        );
        Ok(())
    }

    #[test]
    fn test_run_post_install_rejects_parent_directory_escape()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;

        let source = r#"
class Demo
  def post_install
    (HOMEBREW_PREFIX/".."/".."/"tmp"/"brewdock-owned").mkpath
  end
end
"#;

        let escaped = std::env::temp_dir().join("brewdock-owned");
        let _ = std::fs::remove_dir_all(&escaped);

        let result = run_post_install(source, &mut test_context(&prefix, &keg, "1.0"));

        assert!(
            result.is_err(),
            "post_install path traversal must fail closed before mutating outside prefix"
        );
        assert!(
            !escaped.exists(),
            "post_install must not create directories outside the prefix"
        );
        Ok(())
    }

    #[test]
    fn test_run_post_install_rejects_atomic_write_path_traversal()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;

        let escape = prefix.join("escape.txt");
        let source = r#"
class Demo < Formula
  def post_install
    (etc/"demo"/".."/".."/"escape.txt").atomic_write("owned")
  end
end
"#;

        let result = run_post_install(source, &mut test_context(&prefix, &keg, "1.0"));
        assert!(result.is_err(), "atomic_write traversal should fail closed");
        assert!(
            !escape.exists(),
            "atomic_write traversal should not create files outside allowed roots"
        );
        Ok(())
    }

    #[test]
    fn test_run_post_install_ruby_bundler_cleanup_schema() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/ruby/3.4.2");
        std::fs::create_dir_all(&keg)?;

        let gems_dir = prefix.join("lib/ruby/gems/3.4.0");
        std::fs::create_dir_all(gems_dir.join("bin"))?;
        std::fs::write(gems_dir.join("bin/bundle"), "bundler")?;
        std::fs::write(gems_dir.join("bin/bundler"), "bundler")?;
        std::fs::create_dir_all(gems_dir.join("gems/bundler-2.5.0"))?;
        std::fs::write(gems_dir.join("gems/bundler-2.5.0/fake"), "content")?;
        std::fs::create_dir_all(gems_dir.join("gems/rake-13.0.0"))?;
        std::fs::write(gems_dir.join("gems/rake-13.0.0/keep"), "keep")?;

        let source = r##"
class Ruby < Formula
  def rubygems_bindir
    HOMEBREW_PREFIX/"lib/ruby/gems/#{api_version}/bin"
  end

  def api_version
    "#{version.major.to_i}.#{version.minor.to_i}.0"
  end

  def post_install
    rm(%W[
      #{rubygems_bindir}/bundle
      #{rubygems_bindir}/bundler
    ].select { |file| File.exist?(file) })
    rm_r(Dir[HOMEBREW_PREFIX/"lib/ruby/gems/#{api_version}/gems/bundler-*"])
  end
end
"##;

        let mut context = test_context(&prefix, &keg, "3.4.2");
        run_post_install(source, &mut context)?.commit()?;

        assert!(!gems_dir.join("bin/bundle").exists());
        assert!(!gems_dir.join("bin/bundler").exists());
        assert!(!gems_dir.join("gems/bundler-2.5.0").exists());
        assert!(
            gems_dir.join("gems/rake-13.0.0/keep").exists(),
            "non-bundler gems should be preserved"
        );
        Ok(())
    }

    #[test]
    fn test_run_post_install_node_npm_propagation_schema() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/node/22.0.0");

        std::fs::create_dir_all(keg.join("libexec/lib/node_modules/npm/bin"))?;
        std::fs::write(
            keg.join("libexec/lib/node_modules/npm/bin/npm-cli.js"),
            "npm",
        )?;
        std::fs::write(
            keg.join("libexec/lib/node_modules/npm/bin/npx-cli.js"),
            "npx",
        )?;
        std::fs::create_dir_all(keg.join("libexec/lib/node_modules/npm/man/man1"))?;
        std::fs::write(
            keg.join("libexec/lib/node_modules/npm/man/man1/npm.1"),
            "man",
        )?;
        std::fs::write(
            keg.join("libexec/lib/node_modules/npm/man/man1/package-lock.json.5"),
            "pkg-man",
        )?;
        std::fs::create_dir_all(keg.join("bin"))?;

        let source = r#"
class Node < Formula
  def post_install
    node_modules = HOMEBREW_PREFIX/"lib/node_modules"
    node_modules.mkpath
    rm_r node_modules/"npm" if (node_modules/"npm").exist?

    cp_r libexec/"lib/node_modules/npm", node_modules
    ln_sf node_modules/"npm/bin/npm-cli.js", bin/"npm"
    ln_sf node_modules/"npm/bin/npx-cli.js", bin/"npx"
    ln_sf bin/"npm", HOMEBREW_PREFIX/"bin/npm"
    ln_sf bin/"npx", HOMEBREW_PREFIX/"bin/npx"

    %w[man1 man5 man7].each do |man|
      mkdir_p HOMEBREW_PREFIX/"share/man/#{man}"
      rm(Dir[HOMEBREW_PREFIX/"share/man/#{man}/{npm.,npm-,npmrc.,package.json.,npx.}*"])
      ln_sf Dir[node_modules/"npm/man/#{man}/{npm,package-,shrinkwrap-,npx}*"], HOMEBREW_PREFIX/"share/man/#{man}"
    end

    (node_modules/"npm/npmrc").atomic_write("prefix = #{HOMEBREW_PREFIX}\n")
  end
end
"#;

        let mut context = test_context(&prefix, &keg, "22.0.0");
        run_post_install(source, &mut context)?.commit()?;

        assert!(
            prefix.join("lib/node_modules/npm/bin/npm-cli.js").exists(),
            "npm should be copied to prefix"
        );
        assert!(
            keg.join("bin/npm").is_symlink(),
            "bin/npm should be a symlink"
        );
        assert!(
            keg.join("bin/npx").is_symlink(),
            "bin/npx should be a symlink"
        );
        assert!(
            prefix.join("bin/npm").is_symlink(),
            "prefix bin/npm should be a symlink"
        );
        assert!(
            prefix.join("bin/npx").is_symlink(),
            "prefix bin/npx should be a symlink"
        );

        let npmrc = std::fs::read_to_string(prefix.join("lib/node_modules/npm/npmrc"))?;
        assert!(
            npmrc.starts_with("prefix = "),
            "npmrc should contain prefix setting"
        );
        assert!(
            npmrc.contains(&prefix.to_string_lossy().to_string()),
            "npmrc prefix should point to the actual prefix path"
        );

        assert!(
            prefix.join("share/man/man1").is_dir(),
            "man1 dir should exist"
        );
        let man1_npm = prefix.join("share/man/man1/npm.1");
        assert!(man1_npm.is_symlink(), "npm.1 man page should be symlinked");
        let man1_pkg = prefix.join("share/man/man1/package-lock.json.5");
        assert!(
            man1_pkg.is_symlink(),
            "package-lock.json.5 should be symlinked (matches package- prefix)"
        );

        Ok(())
    }

    #[test]
    fn test_validate_post_install_accepts_ruby_and_node_schemas()
    -> Result<(), Box<dyn std::error::Error>> {
        let ruby_source = r##"
class Ruby < Formula
  def rubygems_bindir
    HOMEBREW_PREFIX/"lib/ruby/gems/#{api_version}/bin"
  end
  def api_version
    "#{version.major.to_i}.#{version.minor.to_i}.0"
  end
  def post_install
    rm(%W[
      #{rubygems_bindir}/bundle
      #{rubygems_bindir}/bundler
    ].select { |file| File.exist?(file) })
    rm_r(Dir[HOMEBREW_PREFIX/"lib/ruby/gems/#{api_version}/gems/bundler-*"])
  end
end
"##;

        let node_source = r#"
class Node < Formula
  def post_install
    node_modules = HOMEBREW_PREFIX/"lib/node_modules"
    node_modules.mkpath
    rm_r node_modules/"npm" if (node_modules/"npm").exist?
    cp_r libexec/"lib/node_modules/npm", node_modules
    ln_sf node_modules/"npm/bin/npm-cli.js", bin/"npm"
    ln_sf node_modules/"npm/bin/npx-cli.js", bin/"npx"
    ln_sf bin/"npm", HOMEBREW_PREFIX/"bin/npm"
    ln_sf bin/"npx", HOMEBREW_PREFIX/"bin/npx"
    %w[man1 man5 man7].each do |man|
      mkdir_p HOMEBREW_PREFIX/"share/man/#{man}"
      rm(Dir[HOMEBREW_PREFIX/"share/man/#{man}/{npm.,npm-,npmrc.,package.json.,npx.}*"])
      ln_sf Dir[node_modules/"npm/man/#{man}/{npm,package-,shrinkwrap-,npx}*"], HOMEBREW_PREFIX/"share/man/#{man}"
    end
    (node_modules/"npm/npmrc").atomic_write("prefix = #{HOMEBREW_PREFIX}\n")
  end
end
"#;

        validate_post_install(ruby_source, "3.4.2")?;
        validate_post_install(node_source, "22.0.0")?;
        Ok(())
    }

    #[test]
    fn test_run_post_install_ignores_ohai_logging() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/fontconfig/2.16.0");
        std::fs::create_dir_all(keg.join("bin"))?;
        std::fs::write(keg.join("bin/fc-cache"), "#!/bin/sh\ntrue\n")?;
        let mut perms = std::fs::metadata(keg.join("bin/fc-cache"))?.permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o755);
        }
        std::fs::set_permissions(keg.join("bin/fc-cache"), perms)?;

        let source = r#"
class Fontconfig < Formula
  def post_install
    ohai "Regenerating font cache, this may take a while"
    system bin/"fc-cache", "--force"
  end
end
"#;

        run_post_install(source, &mut test_context(&prefix, &keg, "2.16.0"))?.commit()?;
        Ok(())
    }

    #[test]
    fn test_run_post_install_handles_homebrew_prefix_constant_path()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/glib/2.88.0");
        std::fs::create_dir_all(&keg)?;

        let source = r#"
class Glib < Formula
  def post_install
    (HOMEBREW_PREFIX/"lib/gio/modules").mkpath
  end
end
"#;

        run_post_install(source, &mut test_context(&prefix, &keg, "2.88.0"))?.commit()?;
        assert!(prefix.join("lib/gio/modules").is_dir());
        Ok(())
    }

    #[test]
    fn test_run_post_install_normalizes_shared_mime_info_schema()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/shared-mime-info/2.4");
        std::fs::create_dir_all(keg.join("bin"))?;
        std::fs::create_dir_all(keg.join("share/shared-mime-info/packages"))?;
        std::fs::write(
            keg.join("share/shared-mime-info/packages/freedesktop.org.xml"),
            "<mime-info/>",
        )?;

        let global_mime = prefix.join("share/mime");
        let stale_target = prefix.join("share/old-mime");
        std::fs::create_dir_all(&stale_target)?;
        std::fs::create_dir_all(global_mime.parent().unwrap_or(&prefix))?;
        std::os::unix::fs::symlink(&stale_target, &global_mime)?;

        let cellar_mime = keg.join("share/mime");
        std::fs::create_dir_all(&cellar_mime)?;
        std::fs::write(cellar_mime.join("stale.cache"), "old")?;

        write_executable(
            &keg.join("bin/update-mime-database"),
            "#!/bin/sh\nmkdir -p \"$1\"\ntouch \"$1/mime.cache\"\n",
        )?;

        run_post_install(
            shared_mime_info_post_install_source(),
            &mut test_context(&prefix, &keg, "2.4"),
        )?
        .commit()?;

        assert!(global_mime.is_dir());
        assert_eq!(
            std::fs::read_to_string(global_mime.join("packages/freedesktop.org.xml"))?,
            "<mime-info/>"
        );
        assert!(global_mime.join("mime.cache").exists());
        assert!(cellar_mime.is_symlink());
        assert_eq!(
            std::fs::read_link(&cellar_mime)?,
            PathBuf::from("../../../../share/mime")
        );
        Ok(())
    }

    #[test]
    fn test_run_post_install_handles_formula_opt_bin_path() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/libheif/1.0");
        std::fs::create_dir_all(&keg)?;

        let shared_mime_keg = prefix.join("Cellar/shared-mime-info/2.4");
        std::fs::create_dir_all(shared_mime_keg.join("bin"))?;
        std::fs::create_dir_all(prefix.join("opt"))?;
        std::os::unix::fs::symlink(
            "../Cellar/shared-mime-info/2.4",
            prefix.join("opt/shared-mime-info"),
        )?;

        write_executable(
            &shared_mime_keg.join("bin/update-mime-database"),
            "#!/bin/sh\nmkdir -p \"$1\"\ntouch \"$1/mime.cache\"\n",
        )?;

        let source = r##"
class Libheif < Formula
  def post_install
    system Formula["shared-mime-info"].opt_bin/"update-mime-database", "#{HOMEBREW_PREFIX}/share/mime"
  end
end
"##;

        run_post_install(source, &mut test_context(&prefix, &keg, "1.0"))?.commit()?;
        assert!(prefix.join("share/mime/mime.cache").exists());
        Ok(())
    }

    #[test]
    fn test_run_post_install_install_and_chmod() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/buildapp/1.5.6");
        std::fs::create_dir_all(keg.join("bin"))?;
        // Pre-create the source file at prefix/buildapp (as if gunzip already ran)
        std::fs::write(keg.join("buildapp"), "#!/bin/sh\necho hello\n")?;

        let source = r#"
class Buildapp < Formula
  def post_install
    bin.install prefix/"buildapp"
    (bin/"buildapp").chmod 0755
  end
end
"#;

        run_post_install(source, &mut test_context(&prefix, &keg, "1.5.6"))?.commit()?;

        // bin.install moves file into bin/
        let installed = keg.join("bin/buildapp");
        assert!(installed.exists(), "bin/buildapp should exist");
        assert!(!keg.join("buildapp").exists(), "source should be removed");
        assert_eq!(
            std::fs::read_to_string(&installed)?,
            "#!/bin/sh\necho hello\n"
        );

        // chmod 0755 sets executable permissions
        let perms = std::fs::metadata(&installed)?.permissions();
        assert_eq!(perms.mode() & 0o777, 0o755);
        Ok(())
    }

    #[test]
    fn test_mirror_tree_creates_symlink_structure() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/demo/1.0");
        std::fs::create_dir_all(keg.join("include/postgresql/server"))?;
        std::fs::write(keg.join("include/postgresql/libpq-fe.h"), "header")?;
        std::fs::write(
            keg.join("include/postgresql/server/pg_config.h"),
            "server header",
        )?;
        std::fs::create_dir_all(&prefix)?;

        let dest_dir = prefix.join("include/demo");
        let mut context = test_context(&prefix, &keg, "1.0");

        // Manually execute MirrorTree
        execute_statements(
            &[Statement::MirrorTree {
                source: PathExpr::new(PathBase::Prefix, &["include", "postgresql"]),
                dest: PathExpr::new(PathBase::HomebrewPrefix, &["include", "demo"]),
                prune_names: vec![".DS_Store".to_owned()],
            }],
            &mut context,
        )?;

        // Check that symlinks were created
        assert!(dest_dir.join("libpq-fe.h").is_symlink());
        assert!(dest_dir.join("server").is_dir());
        assert!(dest_dir.join("server/pg_config.h").is_symlink());

        // Verify symlink targets resolve correctly
        assert_eq!(
            std::fs::read_to_string(dest_dir.join("libpq-fe.h"))?,
            "header"
        );
        assert_eq!(
            std::fs::read_to_string(dest_dir.join("server/pg_config.h"))?,
            "server header"
        );

        Ok(())
    }

    #[test]
    fn test_children_symlink_with_suffix() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/postgresql/17.2");
        std::fs::create_dir_all(keg.join("bin"))?;
        std::fs::write(keg.join("bin/psql"), "#!/bin/sh\n")?;
        std::fs::write(keg.join("bin/pg_dump"), "#!/bin/sh\n")?;
        let link_dir = prefix.join("bin");
        std::fs::create_dir_all(&link_dir)?;

        let mut context = test_context(&prefix, &keg, "17.2");

        execute_statements(
            &[Statement::ChildrenSymlink {
                source_dir: PathExpr::new(PathBase::Bin, &[]),
                link_dir: PathExpr::new(PathBase::HomebrewPrefix, &["bin"]),
                suffix: vec![
                    SegmentPart::Literal("-".to_owned()),
                    SegmentPart::VersionMajor,
                ],
            }],
            &mut context,
        )?;

        assert!(link_dir.join("psql-17").is_symlink());
        assert!(link_dir.join("pg_dump-17").is_symlink());
        assert_eq!(
            std::fs::read_to_string(link_dir.join("psql-17"))?,
            "#!/bin/sh\n"
        );

        Ok(())
    }

    #[test]
    fn test_if_env_skips_when_unset() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;
        std::fs::create_dir_all(&prefix)?;

        let mut context = test_context(&prefix, &keg, "1.0");

        // Use an env var we know is not set
        execute_statements(
            &[Statement::IfEnv {
                variable: "BREWDOCK_DEFINITELY_NOT_SET_12345".to_owned(),
                negate: false,
                then_branch: vec![Statement::Mkpath(PathExpr::new(
                    PathBase::Var,
                    &["test-env"],
                ))],
            }],
            &mut context,
        )?;

        // Branch should NOT have executed since the var is not set
        assert!(!prefix.join("var/test-env").exists());
        Ok(())
    }

    #[test]
    fn test_if_env_negate_executes_when_unset() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;
        std::fs::create_dir_all(&prefix)?;

        let mut context = test_context(&prefix, &keg, "1.0");

        // negate=true with unset var → branch SHOULD execute
        execute_statements(
            &[Statement::IfEnv {
                variable: "BREWDOCK_DEFINITELY_NOT_SET_12345".to_owned(),
                negate: true,
                then_branch: vec![Statement::Mkpath(PathExpr::new(
                    PathBase::Var,
                    &["test-neg"],
                ))],
            }],
            &mut context,
        )?;

        assert!(prefix.join("var/test-neg").is_dir());
        Ok(())
    }

    #[test]
    fn test_process_capture_stores_output() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;
        std::fs::create_dir_all(&prefix)?;

        let mut context = test_context(&prefix, &keg, "1.0");

        execute_statements(
            &[Statement::ProcessCapture {
                variable: "output".to_owned(),
                command: vec![
                    Argument::String("echo".to_owned()),
                    Argument::String("hello world".to_owned()),
                ],
            }],
            &mut context,
        )?;

        assert_eq!(
            context.captured_outputs.get("output"),
            Some(&"hello world".to_owned())
        );

        Ok(())
    }

    #[test]
    fn test_set_env_applies_to_spawned_command() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let prefix = dir.path().join("prefix");
        let keg = prefix.join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;
        std::fs::create_dir_all(&prefix)?;

        let mut context = test_context(&prefix, &keg, "1.0");

        execute_statements(
            &[
                Statement::SetEnv {
                    variable: "BREWDOCK_DEMO_ENV".to_owned(),
                    value: vec![ContentPart::Literal("expected-value".to_owned())],
                },
                Statement::System(vec![
                    Argument::String("sh".to_owned()),
                    Argument::String("-c".to_owned()),
                    Argument::String("[ \"$BREWDOCK_DEMO_ENV\" = \"expected-value\" ]".to_owned()),
                ]),
            ],
            &mut context,
        )?;

        Ok(())
    }
}
