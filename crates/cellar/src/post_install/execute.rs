use std::{ffi::OsString, os::unix::fs::PermissionsExt, path::Path, process::Command};

use brewdock_analysis::{Argument, ContentPart, PathExpr, SegmentPart, Statement};

use super::{
    PostInstallContext,
    context::{install_symlink_path, path_condition_matches},
    rollback::{copy_path, remove_path_if_exists},
};
use crate::{error::CellarError, link::relative_from_to};

pub(super) fn execute_statements(
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
            Statement::GlobChmod { dir, pattern, mode } => {
                let dir_path = context.resolve_allowed_path(dir)?;
                let permissions = std::fs::Permissions::from_mode(*mode);
                if dir_path.is_dir() {
                    for entry in std::fs::read_dir(&dir_path)? {
                        let entry = entry?;
                        if entry
                            .file_name()
                            .to_str()
                            .is_some_and(|name| glob_matches(name, pattern))
                        {
                            std::fs::set_permissions(entry.path(), permissions.clone())?;
                        }
                    }
                }
            }
            Statement::GlobSymlink {
                source_dir,
                pattern,
                link_dir,
            } => execute_glob_symlink(source_dir, pattern, link_dir, context)?,
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
                std::fs::rename(&from_path, &dest).or_else(|_rename_err| {
                    std::fs::copy(&from_path, &dest)?;
                    std::fs::remove_file(&from_path)
                })?;
            }
            Statement::Move { from, to } => {
                let from_path = context.resolve_allowed_path(from)?;
                let to_path = context.resolve_allowed_path(to)?;
                if let Some(parent) = to_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                move_path(&from_path, &to_path)?;
            }
            Statement::MoveChildren { from_dir, to_dir } => {
                execute_move_children(from_dir, to_dir, context)?;
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
            } => execute_children_symlink(source_dir, link_dir, suffix, context)?,
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

fn move_path(from: &Path, to: &Path) -> Result<(), CellarError> {
    std::fs::rename(from, to).or_else(|_| {
        copy_path(from, to)?;
        remove_path_if_exists(from)
    })
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

fn execute_glob_symlink(
    source_dir: &PathExpr,
    pattern: &str,
    link_dir: &PathExpr,
    context: &PostInstallContext,
) -> Result<(), CellarError> {
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
    Ok(())
}

fn execute_move_children(
    from_dir: &PathExpr,
    to_dir: &PathExpr,
    context: &PostInstallContext,
) -> Result<(), CellarError> {
    let from_path = context.resolve_allowed_path(from_dir)?;
    let to_path = context.resolve_allowed_path(to_dir)?;
    std::fs::create_dir_all(&to_path)?;
    if from_path.is_dir() {
        for entry in std::fs::read_dir(&from_path)? {
            let entry = entry?;
            let dest = to_path.join(entry.file_name());
            move_path(&entry.path(), &dest)?;
        }
    }
    Ok(())
}

fn execute_children_symlink(
    source_dir: &PathExpr,
    link_dir: &PathExpr,
    suffix: &[SegmentPart],
    context: &PostInstallContext,
) -> Result<(), CellarError> {
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
    Ok(())
}

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
        if prune_names.iter().any(|p| p == name_str.as_ref()) {
            continue;
        }
        let src = entry.path();
        let relative = src
            .strip_prefix(root)
            .map_err(|err| CellarError::Io(std::io::Error::other(err)))?;
        let dst = dest_root.join(relative);
        if entry.file_type()?.is_dir() {
            if !dst.is_dir() || dst.is_symlink() {
                remove_path_if_exists(&dst)?;
                std::fs::create_dir_all(&dst)?;
            }
            mirror_tree_walk(root, &src, dest_root, prune_names)?;
        } else {
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
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
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
            ContentPart::HomebrewPrefix => result.push_str(&context.prefix.to_string_lossy()),
            ContentPart::Runtime(segment_part) => {
                result.push_str(&context.resolve_segment_part(segment_part));
            }
        }
    }
    result
}
