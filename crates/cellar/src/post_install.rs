use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{error::CellarError, link::relative_from_to};

static ROLLBACK_NONCE: AtomicUsize = AtomicUsize::new(0);

/// Parsed `post_install` program.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Program {
    statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Statement {
    Mkpath(PathExpr),
    Copy {
        from: PathExpr,
        to: PathExpr,
    },
    System(Vec<Argument>),
    IfExists {
        condition: PathExpr,
        then_branch: Vec<Self>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Argument {
    Path(PathExpr),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PathExpr {
    base: PathBase,
    segments: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PathBase {
    Prefix,
    Bin,
    Etc,
    Lib,
    Pkgetc,
    Pkgshare,
    Sbin,
    Share,
    Var,
}

/// Execution environment for the restricted `post_install` DSL.
#[derive(Debug, Clone)]
pub struct PostInstallContext {
    formula_name: String,
    prefix: PathBuf,
    keg_path: PathBuf,
}

/// Rollback handle for a completed `post_install` execution.
#[derive(Debug)]
pub struct PostInstallTransaction {
    backups: Vec<(PathBuf, Option<PathBuf>)>,
    rollback_dir: PathBuf,
}

impl PostInstallContext {
    /// Creates a new context for a materialized keg.
    #[must_use]
    pub fn new(prefix: &Path, keg_path: &Path) -> Self {
        Self {
            formula_name: formula_name_from_keg(keg_path),
            prefix: prefix.to_path_buf(),
            keg_path: keg_path.to_path_buf(),
        }
    }

    fn resolve_path(&self, expr: &PathExpr) -> PathBuf {
        let mut path = match expr.base {
            PathBase::Prefix => self.keg_path.clone(),
            PathBase::Bin => self.keg_path.join("bin"),
            PathBase::Etc => self.prefix.join("etc"),
            PathBase::Lib => self.keg_path.join("lib"),
            PathBase::Pkgetc => self.prefix.join("etc").join(&self.formula_name),
            PathBase::Pkgshare => self.keg_path.join("share").join(&self.formula_name),
            PathBase::Share => self.keg_path.join("share"),
            PathBase::Sbin => self.keg_path.join("sbin"),
            PathBase::Var => self.prefix.join("var"),
        };
        for segment in &expr.segments {
            path.push(segment);
        }
        path
    }
}

fn formula_name_from_keg(keg_path: &Path) -> String {
    keg_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map_or_else(String::new, ToOwned::to_owned)
}

/// Extracts the contents of `def post_install ... end`.
///
/// # Errors
///
/// Returns [`CellarError::UnsupportedPostInstallSyntax`] when the method is
/// missing or block boundaries cannot be matched.
pub fn extract_post_install_block(source: &str) -> Result<String, CellarError> {
    let mut in_post_install = false;
    let mut depth = 0_usize;
    let mut block_lines = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        if !in_post_install {
            if trimmed == "def post_install" {
                in_post_install = true;
                depth = 1;
            }
            continue;
        }

        if opens_block(trimmed) {
            depth += 1;
        }

        if trimmed == "end" {
            depth =
                depth
                    .checked_sub(1)
                    .ok_or_else(|| CellarError::UnsupportedPostInstallSyntax {
                        message: "unexpected end while extracting post_install".to_owned(),
                    })?;
            if depth == 0 {
                return Ok(block_lines.join("\n"));
            }
        }

        block_lines.push(line.to_owned());
    }

    Err(CellarError::UnsupportedPostInstallSyntax {
        message: "missing def post_install block".to_owned(),
    })
}

/// Executes a restricted `post_install` block.
///
/// Supported statements:
/// - `(path).mkpath`
/// - `cp from, to`
/// - `system command, args...`
/// - `if (path).exist? ... end`
///
/// # Errors
///
/// Returns [`CellarError::UnsupportedPostInstallSyntax`] for any unsupported
/// Ruby construct and [`CellarError::PostInstallCommandFailed`] when a spawned
/// command exits unsuccessfully.
pub fn run_post_install(
    source: &str,
    context: &PostInstallContext,
) -> Result<PostInstallTransaction, CellarError> {
    if let Some(transaction) = run_builtin_post_install(source, context)? {
        return Ok(transaction);
    }
    let block = extract_post_install_block(source)?;
    let program = parse_program(&block)?;
    let rollback_roots = collect_rollback_roots(&program, context);
    run_with_rollback(&rollback_roots, || {
        execute_statements(&program.statements, context)
    })
}

fn run_builtin_post_install(
    source: &str,
    context: &PostInstallContext,
) -> Result<Option<PostInstallTransaction>, CellarError> {
    if matches_certificate_bundle_bootstrap(source) {
        let rollback_roots = vec![context.prefix.join("etc").join(&context.formula_name)];
        return run_with_rollback(&rollback_roots, || {
            run_certificate_bundle_bootstrap(context)
        })
        .map(Some);
    }

    if matches_openssl_cert_symlink_bootstrap(source) {
        let openssldir_name = parse_openssldir_name(source)?;
        let rollback_roots = vec![context.prefix.join("etc").join(openssldir_name)];
        return run_with_rollback(&rollback_roots, || {
            run_openssl_cert_symlink_bootstrap(openssldir_name, context)
        })
        .map(Some);
    }

    Ok(None)
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

fn matches_certificate_bundle_bootstrap(source: &str) -> bool {
    source.contains("def post_install")
        && source.contains("if OS.mac?")
        && source.contains("macos_post_install")
        && source.contains("linux_post_install")
        && source.contains(r#"pkgshare/"cacert.pem""#)
        && source.contains(r#"pkgetc/"cert.pem""#)
}

fn run_certificate_bundle_bootstrap(context: &PostInstallContext) -> Result<(), CellarError> {
    let source_bundle = context
        .keg_path
        .join("share")
        .join(&context.formula_name)
        .join("cacert.pem");
    let target_bundle = context
        .prefix
        .join("etc")
        .join(&context.formula_name)
        .join("cert.pem");
    if let Some(parent) = target_bundle.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let bundle = std::fs::read(&source_bundle)?;
    std::fs::write(&target_bundle, bundle)?;
    Ok(())
}

fn matches_openssl_cert_symlink_bootstrap(source: &str) -> bool {
    source.contains("def openssldir")
        && source.contains(r#"rm(openssldir/"cert.pem") if (openssldir/"cert.pem").exist?"#)
        && source.contains(r#"install_symlink Formula["ca-certificates"].pkgetc/"cert.pem""#)
}

fn run_openssl_cert_symlink_bootstrap(
    openssldir_name: &str,
    context: &PostInstallContext,
) -> Result<(), CellarError> {
    let openssldir = context.prefix.join("etc").join(openssldir_name);
    std::fs::create_dir_all(&openssldir)?;

    let cert_link = openssldir.join("cert.pem");
    if cert_link.symlink_metadata().is_ok() {
        std::fs::remove_file(&cert_link)?;
    }

    let ca_cert = context
        .prefix
        .join("etc")
        .join("ca-certificates")
        .join("cert.pem");
    let link_target = relative_from_to(&openssldir, &ca_cert);
    std::os::unix::fs::symlink(link_target, cert_link)?;
    Ok(())
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
            Statement::Mkpath(path) => {
                if let Some(root) = rollback_root(&context.resolve_path(path), context) {
                    roots.insert(root);
                }
            }
            Statement::Copy { to, .. } => {
                if let Some(root) = rollback_root(&context.resolve_path(to), context) {
                    roots.insert(root);
                }
            }
            Statement::System(arguments) => {
                for argument in arguments.iter().skip(1) {
                    if let Argument::Path(path) = argument
                        && let Some(root) = rollback_root(&context.resolve_path(path), context)
                    {
                        roots.insert(root);
                    }
                }
            }
            Statement::IfExists { then_branch, .. } => {
                collect_statement_roots(then_branch, context, roots);
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
    run: F,
) -> Result<PostInstallTransaction, CellarError>
where
    F: FnOnce() -> Result<(), CellarError>,
{
    let rollback_dir = make_rollback_dir()?;
    let backups = rollback_roots
        .iter()
        .map(|root| {
            let backup = if root.symlink_metadata().is_ok() {
                let backup = rollback_dir.join(format!("entry-{}", backups_len_hint(root)));
                copy_path(root, &backup)?;
                Some(backup)
            } else {
                None
            };
            Ok((root.clone(), backup))
        })
        .collect::<Result<Vec<_>, CellarError>>()?;

    match run() {
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

fn backups_len_hint(path: &Path) -> usize {
    path.components().count() + ROLLBACK_NONCE.fetch_add(1, Ordering::Relaxed)
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

fn parse_openssldir_name(source: &str) -> Result<&str, CellarError> {
    source
        .lines()
        .find_map(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix("etc/\"")
                .and_then(|rest| rest.strip_suffix('\"'))
        })
        .ok_or_else(|| CellarError::UnsupportedPostInstallSyntax {
            message: "could not parse openssldir path".to_owned(),
        })
}

fn opens_block(trimmed: &str) -> bool {
    trimmed.starts_with("if ")
}

fn parse_program(block: &str) -> Result<Program, CellarError> {
    let lines: Vec<&str> = block.lines().collect();
    let mut index = 0;
    let statements = parse_statements(&lines, &mut index)?;
    Ok(Program { statements })
}

fn parse_statements(lines: &[&str], index: &mut usize) -> Result<Vec<Statement>, CellarError> {
    let mut statements = Vec::new();

    while *index < lines.len() {
        let line = lines[*index].trim();
        *index += 1;

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line == "end" {
            break;
        }

        if let Some(condition) = line
            .strip_prefix("if ")
            .and_then(|rest| rest.strip_suffix(".exist?"))
        {
            let condition = parse_path_expr(condition.trim())?;
            let then_branch = parse_statements(lines, index)?;
            statements.push(Statement::IfExists {
                condition,
                then_branch,
            });
            continue;
        }

        if let Some(path) = line.strip_suffix(".mkpath") {
            statements.push(Statement::Mkpath(parse_path_expr(path.trim())?));
            continue;
        }

        if let Some(args) = line.strip_prefix("cp ") {
            let parts = split_args(args)?;
            if parts.len() != 2 {
                return unsupported("cp expects exactly two arguments");
            }
            statements.push(Statement::Copy {
                from: parse_path_expr(parts[0].trim())?,
                to: parse_path_expr(parts[1].trim())?,
            });
            continue;
        }

        if let Some(args) = line.strip_prefix("system ") {
            let parts = split_args(args)?;
            if parts.is_empty() {
                return unsupported("system expects at least one argument");
            }
            let args = parts
                .iter()
                .map(|part| parse_argument(part))
                .collect::<Result<Vec<_>, _>>()?;
            statements.push(Statement::System(args));
            continue;
        }

        return unsupported(&format!("unsupported post_install statement: {line}"));
    }

    Ok(statements)
}

fn parse_argument(raw: &str) -> Result<Argument, CellarError> {
    let trimmed = raw.trim();
    if let Some(value) = parse_string_literal(trimmed) {
        return Ok(Argument::String(value));
    }
    Ok(Argument::Path(parse_path_expr(trimmed)?))
}

fn parse_path_expr(raw: &str) -> Result<PathExpr, CellarError> {
    let trimmed = raw.trim().trim_start_matches('(').trim_end_matches(')');
    let Some(split_index) = find_path_split(trimmed) else {
        return Ok(PathExpr {
            base: parse_path_base(trimmed.trim())?,
            segments: Vec::new(),
        });
    };
    let (base, rest) = trimmed.split_at(split_index);
    let base = base.trim();
    let base = parse_path_base(base)?;

    let segments = parse_path_segments(rest)?;

    Ok(PathExpr { base, segments })
}

fn find_path_split(raw: &str) -> Option<usize> {
    let mut in_string = false;
    for (index, ch) in raw.char_indices() {
        match ch {
            '"' => in_string = !in_string,
            '/' if !in_string => return Some(index),
            _ => {}
        }
    }
    None
}

fn parse_path_base(base: &str) -> Result<PathBase, CellarError> {
    match base {
        "prefix" => Ok(PathBase::Prefix),
        "bin" => Ok(PathBase::Bin),
        "etc" => Ok(PathBase::Etc),
        "lib" => Ok(PathBase::Lib),
        "pkgetc" => Ok(PathBase::Pkgetc),
        "pkgshare" => Ok(PathBase::Pkgshare),
        "sbin" => Ok(PathBase::Sbin),
        "share" => Ok(PathBase::Share),
        "var" => Ok(PathBase::Var),
        _ => unsupported(&format!("unsupported path base: {base}")),
    }
}

fn parse_path_segments(raw: &str) -> Result<Vec<String>, CellarError> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }

    let mut segments = Vec::new();
    let mut remainder = raw.trim();
    while !remainder.is_empty() {
        let without_slash = remainder.strip_prefix('/').ok_or_else(|| {
            CellarError::UnsupportedPostInstallSyntax {
                message: format!("unsupported path segment: {remainder}"),
            }
        })?;
        let Some(stripped) = without_slash.strip_prefix('"') else {
            return unsupported(&format!("unsupported path segment: {remainder}"));
        };
        let end = stripped
            .find('"')
            .ok_or_else(|| CellarError::UnsupportedPostInstallSyntax {
                message: format!("unterminated string literal in path: {remainder}"),
            })?;
        segments.push(stripped[..end].to_owned());
        remainder = stripped[end + 1..].trim();
    }

    Ok(segments)
}

fn parse_string_literal(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        return Some(trimmed[1..trimmed.len() - 1].to_owned());
    }
    None
}

fn split_args(raw: &str) -> Result<Vec<String>, CellarError> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut paren_depth = 0_i32;

    for ch in raw.chars() {
        match ch {
            '"' => {
                in_string = !in_string;
                current.push(ch);
            }
            '(' if !in_string => {
                paren_depth += 1;
                current.push(ch);
            }
            ')' if !in_string => {
                paren_depth -= 1;
                current.push(ch);
            }
            ',' if !in_string && paren_depth == 0 => {
                parts.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    if in_string || paren_depth != 0 {
        return unsupported("unterminated post_install argument list");
    }

    if !current.trim().is_empty() {
        parts.push(current.trim().to_owned());
    }

    Ok(parts)
}

fn execute_statements(
    statements: &[Statement],
    context: &PostInstallContext,
) -> Result<(), CellarError> {
    for statement in statements {
        match statement {
            Statement::Mkpath(path) => {
                std::fs::create_dir_all(context.resolve_path(path))?;
            }
            Statement::Copy { from, to } => {
                let from = context.resolve_path(from);
                let to = context.resolve_path(to);
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(from, to)?;
            }
            Statement::System(arguments) => run_system(arguments, context)?,
            Statement::IfExists {
                condition,
                then_branch,
            } => {
                if context.resolve_path(condition).exists() {
                    execute_statements(then_branch, context)?;
                }
            }
        }
    }
    Ok(())
}

fn run_system(arguments: &[Argument], context: &PostInstallContext) -> Result<(), CellarError> {
    let command_line = arguments
        .iter()
        .map(|arg| match arg {
            Argument::Path(path) => Ok(context.resolve_path(path).into_os_string()),
            Argument::String(value) => Ok(OsString::from(value)),
        })
        .collect::<Result<Vec<_>, CellarError>>()?;

    let (program, program_args) =
        command_line
            .split_first()
            .ok_or_else(|| CellarError::UnsupportedPostInstallSyntax {
                message: "system expects at least one argument".to_owned(),
            })?;
    let output = Command::new(program).args(program_args).output()?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(CellarError::PostInstallCommandFailed {
        message: if stderr.trim().is_empty() {
            format!("exit status {}", output.status)
        } else {
            stderr.trim().to_owned()
        },
    })
}

fn unsupported<T>(message: &str) -> Result<T, CellarError> {
    Err(CellarError::UnsupportedPostInstallSyntax {
        message: message.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

        run_post_install(source, &PostInstallContext::new(&prefix, &keg))?.commit()?;

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

        let result = run_post_install(source, &PostInstallContext::new(&prefix, &keg));
        assert!(matches!(
            result,
            Err(CellarError::UnsupportedPostInstallSyntax { .. })
        ));
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

        run_post_install(source, &PostInstallContext::new(&prefix, &keg))?.commit()?;

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

        run_post_install(source, &PostInstallContext::new(&prefix, &keg))?.commit()?;

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

        let result = run_post_install(source, &PostInstallContext::new(&prefix, &keg));

        assert!(matches!(
            result,
            Err(CellarError::PostInstallCommandFailed { .. })
        ));
        assert!(!prefix.join("var/demo").exists());
        Ok(())
    }
}
