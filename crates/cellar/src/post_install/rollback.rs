use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use brewdock_analysis::{Argument, Program, Statement};

use super::{PostInstallContext, PostInstallTransaction, context::install_symlink_path};
use crate::error::CellarError;

static ROLLBACK_NONCE: AtomicUsize = AtomicUsize::new(0);

pub(super) fn collect_rollback_roots(
    program: &Program,
    context: &PostInstallContext,
) -> Vec<PathBuf> {
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
            | Statement::GlobChmod { dir: to, .. }
            | Statement::GlobSymlink { link_dir: to, .. }
            | Statement::ForceSymlink { link: to, .. }
            | Statement::Install { into_dir: to, .. }
            | Statement::Move { to, .. }
            | Statement::MoveChildren { to_dir: to, .. }
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
            Statement::ProcessCapture { .. } | Statement::SetEnv { .. } => {}
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
    let mut collapsed: Vec<PathBuf> = Vec::new();
    for root in roots {
        // Sorted order guarantees any ancestor appears immediately before its
        // descendants, so checking only `last()` is equivalent to `.any()`.
        if collapsed
            .last()
            .is_some_and(|parent| root.starts_with(parent))
        {
            continue;
        }
        collapsed.push(root);
    }
    collapsed
}

pub(super) fn run_with_rollback<F>(
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

pub(super) fn restore_backups(backups: &[(PathBuf, Option<PathBuf>)]) -> Result<(), CellarError> {
    for (root, backup) in backups {
        remove_path_if_exists(root)?;
        if let Some(backup) = backup {
            copy_path(backup, root)?;
        }
    }
    Ok(())
}

pub(super) fn remove_path_if_exists(path: &Path) -> Result<(), CellarError> {
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

pub(super) fn copy_path(from: &Path, to: &Path) -> Result<(), CellarError> {
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
