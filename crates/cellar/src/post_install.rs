use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

use ruby_prism::{ConstantId, Node, ParseResult, parse as parse_ruby};

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
    RemoveIfExists(PathExpr),
    InstallSymlink {
        link_dir: PathExpr,
        target: PathExpr,
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
    FormulaPkgetc(String),
    Lib,
    Pkgetc,
    Pkgshare,
    Sbin,
    Share,
    Var,
}

#[derive(Debug)]
struct MethodDef<'pr> {
    body: Option<Node<'pr>>,
    has_receiver: bool,
    has_parameters: bool,
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
            PathBase::FormulaPkgetc(ref formula) => self.prefix.join("etc").join(formula),
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
    let parsed = parse_source(source)?;
    let methods = build_method_table(&parsed)?;
    let method =
        methods
            .get("post_install")
            .ok_or_else(|| CellarError::UnsupportedPostInstallSyntax {
                message: "missing def post_install block".to_owned(),
            })?;
    let body = method
        .body
        .as_ref()
        .ok_or_else(|| CellarError::UnsupportedPostInstallSyntax {
            message: "missing def post_install block".to_owned(),
        })?;

    node_source(&parsed, body).map(ToOwned::to_owned)
}

/// Executes a restricted `post_install` block.
///
/// Supported statements:
/// - `(path).mkpath`
/// - `cp from, to`
/// - `system command, args...`
/// - `if (path).exist? ... end`
///
/// Validates that a `post_install` block can be parsed and lowered without executing it.
///
/// Use this to check whether a formula's `post_install` is supported before
/// performing destructive operations like unlinking the old keg during upgrade.
///
/// # Errors
///
/// Returns [`CellarError::UnsupportedPostInstallSyntax`] if the source contains
/// unsupported Ruby constructs that cannot be lowered.
pub fn validate_post_install(source: &str) -> Result<(), CellarError> {
    let _program = lower_post_install(source)?;
    Ok(())
}

/// # Errors
///
/// Returns [`CellarError::UnsupportedPostInstallSyntax`] for any unsupported
/// Ruby construct and [`CellarError::PostInstallCommandFailed`] when a spawned
/// command exits unsuccessfully.
pub fn run_post_install(
    source: &str,
    context: &PostInstallContext,
) -> Result<PostInstallTransaction, CellarError> {
    let program = lower_post_install(source)?;
    let rollback_roots = collect_rollback_roots(&program, context);
    run_with_rollback(&rollback_roots, || {
        execute_statements(&program.statements, context)
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
            Statement::Copy { to, .. } => {
                if let Some(root) = rollback_root(&context.resolve_path(to), context) {
                    roots.insert(root);
                }
            }
            Statement::Mkpath(path) | Statement::RemoveIfExists(path) => {
                if let Some(root) = rollback_root(&context.resolve_path(path), context) {
                    roots.insert(root);
                }
            }
            Statement::InstallSymlink { link_dir, target } => {
                if let Ok(link_path) = install_symlink_path(
                    &context.resolve_path(link_dir),
                    &context.resolve_path(target),
                ) && let Some(root) = rollback_root(&link_path, context)
                {
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

fn lower_post_install(source: &str) -> Result<Program, CellarError> {
    let parsed = parse_source(source)?;
    let methods = build_method_table(&parsed)?;
    let mut helper_stack = BTreeSet::new();
    let statements = lower_method("post_install", &parsed, &methods, &mut helper_stack)?;
    Ok(Program { statements })
}

fn parse_source(source: &str) -> Result<ParseResult<'_>, CellarError> {
    let parsed = parse_ruby(source.as_bytes());
    if let Some(error) = parsed.errors().next() {
        return unsupported(&format!("prism parse error: {}", error.message()));
    }
    Ok(parsed)
}

fn build_method_table<'pr>(
    parsed: &'pr ParseResult<'pr>,
) -> Result<BTreeMap<String, MethodDef<'pr>>, CellarError> {
    let mut methods = BTreeMap::new();
    collect_methods_from_node(&parsed.node(), &mut methods)?;
    Ok(methods)
}

fn collect_methods_from_node<'pr>(
    node: &Node<'pr>,
    methods: &mut BTreeMap<String, MethodDef<'pr>>,
) -> Result<(), CellarError> {
    if let Some(program) = node.as_program_node() {
        for child in &program.statements().body() {
            collect_methods_from_node(&child, methods)?;
        }
        return Ok(());
    }

    if let Some(statements) = node.as_statements_node() {
        for child in &statements.body() {
            collect_methods_from_node(&child, methods)?;
        }
        return Ok(());
    }

    if let Some(class) = node.as_class_node() {
        if let Some(body) = class.body() {
            collect_methods_from_node(&body, methods)?;
        }
        return Ok(());
    }

    if let Some(def) = node.as_def_node() {
        let name_id = def.name();
        let name = constant_name(&name_id)?;
        methods.insert(
            name,
            MethodDef {
                body: def.body(),
                has_receiver: def.receiver().is_some(),
                has_parameters: def.parameters().is_some(),
            },
        );
    }

    Ok(())
}

fn lower_method<'pr>(
    name: &str,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, CellarError> {
    let method = methods
        .get(name)
        .ok_or_else(|| CellarError::UnsupportedPostInstallSyntax {
            message: format!("unknown helper method: {name}"),
        })?;

    if method.has_receiver || method.has_parameters {
        return unsupported(&format!("unsupported helper method signature: {name}"));
    }

    if !helper_stack.insert(name.to_owned()) {
        return unsupported(&format!("recursive helper method: {name}"));
    }

    let result = (|| {
        let Some(body) = method.body.as_ref() else {
            return Ok(Vec::new());
        };
        if let Some(statements) = normalize_method_schema(body, parsed, methods, helper_stack)? {
            return Ok(statements);
        }
        lower_body_node(body, parsed, methods, helper_stack)
    })();

    helper_stack.remove(name);
    result
}

fn normalize_method_schema<'pr>(
    body: &Node<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Option<Vec<Statement>>, CellarError> {
    if matches_bundle_bootstrap_schema(body, parsed, methods, helper_stack)? {
        return Ok(Some(vec![
            Statement::Mkpath(PathExpr {
                base: PathBase::Pkgetc,
                segments: Vec::new(),
            }),
            Statement::Copy {
                from: PathExpr {
                    base: PathBase::Pkgshare,
                    segments: vec!["cacert.pem".to_owned()],
                },
                to: PathExpr {
                    base: PathBase::Pkgetc,
                    segments: vec!["cert.pem".to_owned()],
                },
            },
        ]));
    }

    if let Some((link_dir, target)) =
        detect_cert_symlink_schema(body, parsed, methods, helper_stack)?
    {
        return Ok(Some(vec![
            Statement::RemoveIfExists(append_segment(&link_dir, "cert.pem")),
            Statement::InstallSymlink { link_dir, target },
        ]));
    }

    Ok(None)
}

fn matches_bundle_bootstrap_schema<'pr>(
    body: &Node<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<bool, CellarError> {
    let mut saw_mkpath = false;
    let mut saw_atomic_write = false;
    visit_calls(body, &mut |call| {
        let name = call_name(call)?;
        if name == "mkpath" {
            if let Some(receiver) = call.receiver()
                && parse_path_expr(&receiver, parsed, methods, helper_stack)
                    .is_ok_and(|path| path.base == PathBase::Pkgetc && path.segments.is_empty())
            {
                saw_mkpath = true;
            }
        } else if name == "atomic_write"
            && let Some(receiver) = call.receiver()
            && parse_path_expr(&receiver, parsed, methods, helper_stack)
                .is_ok_and(|path| path.base == PathBase::Pkgetc && path.segments == ["cert.pem"])
        {
            saw_atomic_write = true;
        }
        Ok(())
    })?;

    Ok(saw_mkpath && saw_atomic_write)
}

fn detect_cert_symlink_schema<'pr>(
    body: &Node<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Option<(PathExpr, PathExpr)>, CellarError> {
    let mut link_dir = None;
    let mut target = None;

    for statement in body_statements(body)? {
        if let Some(call) = statement.as_call_node() {
            let name = call_name(&call)?;
            if name == "install_symlink"
                && let Some(receiver) = call.receiver()
            {
                let receiver = parse_path_expr(&receiver, parsed, methods, helper_stack)?;
                let arguments = call_args(&call);
                if arguments.len() == 1 {
                    link_dir = Some(receiver);
                    target = Some(parse_path_expr(
                        &arguments[0],
                        parsed,
                        methods,
                        helper_stack,
                    )?);
                }
            }
        }
    }

    Ok(link_dir.zip(target))
}

fn lower_body_node<'pr>(
    body: &Node<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, CellarError> {
    let mut statements = Vec::new();
    for child in body_statements(body)? {
        statements.extend(lower_statement(&child, parsed, methods, helper_stack)?);
    }
    Ok(statements)
}

fn body_statements<'pr>(body: &Node<'pr>) -> Result<Vec<Node<'pr>>, CellarError> {
    if let Some(statements) = body.as_statements_node() {
        return Ok(statements.body().iter().collect());
    }

    if let Some(begin) = body.as_begin_node() {
        if begin.rescue_clause().is_some()
            || begin.else_clause().is_some()
            || begin.ensure_clause().is_some()
        {
            return unsupported("unsupported begin/rescue/ensure in runtime branch");
        }
        return begin.statements().map_or_else(
            || Ok(Vec::new()),
            |statements| Ok(statements.body().iter().collect()),
        );
    }

    unsupported("unsupported method body container")
}

fn lower_statement<'pr>(
    node: &Node<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, CellarError> {
    if let Some(if_node) = node.as_if_node() {
        return lower_if_statement(&if_node, parsed, methods, helper_stack);
    }

    if let Some(call) = node.as_call_node() {
        return lower_call_statement(&call, parsed, methods, helper_stack);
    }

    unsupported(&format!(
        "unsupported post_install statement: {}",
        node_source(parsed, node)?
    ))
}

fn lower_if_statement<'pr>(
    if_node: &ruby_prism::IfNode<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, CellarError> {
    if predicate_is_os_runtime(&if_node.predicate(), "mac?")? {
        return lower_statements_node_opt(if_node.statements(), parsed, methods, helper_stack);
    }

    if predicate_is_os_runtime(&if_node.predicate(), "linux?")? {
        return lower_else_branch(if_node.subsequent(), parsed, methods, helper_stack);
    }

    if let Some(condition) =
        parse_exist_condition(&if_node.predicate(), parsed, methods, helper_stack)?
    {
        let then_branch =
            lower_statements_node_opt(if_node.statements(), parsed, methods, helper_stack)?;
        if if_node.subsequent().is_none()
            && then_branch.len() == 1
            && matches!(then_branch.first(), Some(Statement::RemoveIfExists(path)) if *path == condition)
        {
            return Ok(then_branch);
        }
        if if_node.subsequent().is_some() {
            return unsupported("unsupported else branch for path existence condition");
        }
        return Ok(vec![Statement::IfExists {
            condition,
            then_branch,
        }]);
    }

    unsupported(&format!(
        "unsupported post_install conditional: {}",
        node_source(parsed, &if_node.predicate())?
    ))
}

fn lower_else_branch<'pr>(
    subsequent: Option<Node<'pr>>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, CellarError> {
    let Some(subsequent) = subsequent else {
        return Ok(Vec::new());
    };
    if let Some(else_node) = subsequent.as_else_node() {
        return lower_statements_node_opt(else_node.statements(), parsed, methods, helper_stack);
    }
    if let Some(if_node) = subsequent.as_if_node() {
        return lower_if_statement(&if_node, parsed, methods, helper_stack);
    }
    unsupported("unsupported runtime else branch")
}

fn lower_statements_node_opt<'pr>(
    statements: Option<ruby_prism::StatementsNode<'pr>>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, CellarError> {
    let Some(statements) = statements else {
        return Ok(Vec::new());
    };
    let mut lowered = Vec::new();
    for child in &statements.body() {
        lowered.extend(lower_statement(&child, parsed, methods, helper_stack)?);
    }
    Ok(lowered)
}

fn lower_call_statement<'pr>(
    call: &ruby_prism::CallNode<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, CellarError> {
    let name = call_name(call)?;
    if let Some(receiver) = call.receiver() {
        let receiver = parse_path_expr(&receiver, parsed, methods, helper_stack)?;
        return match name.as_str() {
            "mkpath" => Ok(vec![Statement::Mkpath(receiver)]),
            "install_symlink" => {
                let arguments = call_args(call);
                if arguments.len() != 1 {
                    return unsupported("install_symlink expects exactly one argument");
                }
                Ok(vec![Statement::InstallSymlink {
                    link_dir: receiver,
                    target: parse_path_expr(&arguments[0], parsed, methods, helper_stack)?,
                }])
            }
            _ => unsupported(&format!("unsupported post_install call: {name}")),
        };
    }

    match name.as_str() {
        "cp" => {
            let arguments = call_args(call);
            if arguments.len() != 2 {
                return unsupported("cp expects exactly two arguments");
            }
            Ok(vec![Statement::Copy {
                from: parse_path_expr(&arguments[0], parsed, methods, helper_stack)?,
                to: parse_path_expr(&arguments[1], parsed, methods, helper_stack)?,
            }])
        }
        "system" => {
            let arguments = call_args(call);
            if arguments.is_empty() {
                return unsupported("system expects at least one argument");
            }
            let arguments = arguments
                .iter()
                .map(|argument| parse_argument(argument, parsed, methods, helper_stack))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(vec![Statement::System(arguments)])
        }
        "rm" => {
            let arguments = call_args(call);
            if arguments.is_empty() || arguments.len() > 2 {
                return unsupported("rm expects one path argument");
            }
            if arguments.len() == 2 && !is_force_true_keyword(&arguments[1], parsed)? {
                return unsupported("rm only supports force: true keyword");
            }
            Ok(vec![Statement::RemoveIfExists(parse_path_expr(
                &arguments[0],
                parsed,
                methods,
                helper_stack,
            )?)])
        }
        helper if methods.contains_key(helper) => {
            lower_method(helper, parsed, methods, helper_stack)
        }
        _ => unsupported(&format!("unsupported post_install statement: {name}")),
    }
}

fn parse_argument<'pr>(
    node: &Node<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Argument, CellarError> {
    if let Some(value) = parse_string(node)? {
        return Ok(Argument::String(value));
    }
    Ok(Argument::Path(parse_path_expr(
        node,
        parsed,
        methods,
        helper_stack,
    )?))
}

fn parse_path_expr<'pr>(
    node: &Node<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<PathExpr, CellarError> {
    if let Ok(path) = parse_path_expr_ast(node, parsed, methods, helper_stack) {
        return Ok(path);
    }
    parse_path_expr_text(node_source(parsed, node)?)
}

fn parse_path_expr_ast<'pr>(
    node: &Node<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<PathExpr, CellarError> {
    if let Some(parentheses) = node.as_parentheses_node()
        && let Some(body) = parentheses.body()
    {
        return parse_path_expr_ast(&body, parsed, methods, helper_stack);
    }

    let Some(call) = node.as_call_node() else {
        return unsupported(&format!(
            "unsupported path expression: {}",
            node_source(parsed, node)?
        ));
    };
    let name = call_name(&call)?;
    if name == "/" {
        let Some(receiver) = call.receiver() else {
            return unsupported("path join requires receiver");
        };
        let mut path = parse_path_expr_ast(&receiver, parsed, methods, helper_stack)?;
        let arguments = call_args(&call);
        if arguments.len() != 1 {
            return unsupported("path join expects exactly one segment");
        }
        let Some(segment) = parse_string(&arguments[0])? else {
            return unsupported("path join segment must be string literal");
        };
        path.segments.push(segment);
        return Ok(path);
    }

    if name == "pkgetc"
        && let Some(receiver) = call.receiver()
        && let Some(formula) = parse_formula_ref(&receiver)?
    {
        return Ok(PathExpr {
            base: PathBase::FormulaPkgetc(formula),
            segments: Vec::new(),
        });
    }

    if call.receiver().is_none() && call.arguments().is_none() {
        if let Some(base) = parse_path_base(&name) {
            return Ok(PathExpr {
                base,
                segments: Vec::new(),
            });
        }
        return parse_helper_path_expr(&name, parsed, methods, helper_stack);
    }

    unsupported(&format!(
        "unsupported path expression: {}",
        node_source(parsed, node)?
    ))
}

fn parse_helper_path_expr<'pr>(
    name: &str,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<PathExpr, CellarError> {
    let method = methods
        .get(name)
        .ok_or_else(|| CellarError::UnsupportedPostInstallSyntax {
            message: format!("unsupported path base: {name}"),
        })?;
    if method.has_receiver || method.has_parameters {
        return unsupported(&format!("unsupported path helper signature: {name}"));
    }
    if !helper_stack.insert(format!("path:{name}")) {
        return unsupported(&format!("recursive path helper: {name}"));
    }
    let result = (|| {
        let Some(body) = method.body.as_ref() else {
            return unsupported(&format!("empty path helper: {name}"));
        };
        let statements = body_statements(body)?;
        if statements.len() != 1 {
            return unsupported(&format!("path helper must lower to one expression: {name}"));
        }
        parse_path_expr(&statements[0], parsed, methods, helper_stack)
    })();
    helper_stack.remove(&format!("path:{name}"));
    result
}

fn parse_path_base(name: &str) -> Option<PathBase> {
    match name {
        "prefix" => Some(PathBase::Prefix),
        "bin" => Some(PathBase::Bin),
        "etc" => Some(PathBase::Etc),
        "lib" => Some(PathBase::Lib),
        "pkgetc" => Some(PathBase::Pkgetc),
        "pkgshare" => Some(PathBase::Pkgshare),
        "sbin" => Some(PathBase::Sbin),
        "share" => Some(PathBase::Share),
        "var" => Some(PathBase::Var),
        _ => None,
    }
}

fn parse_path_expr_text(raw: &str) -> Result<PathExpr, CellarError> {
    let trimmed = raw.trim().trim_start_matches('(').trim_end_matches(')');
    let Some(split_index) = find_path_split(trimmed) else {
        return parse_path_base(trimmed)
            .map(|base| PathExpr {
                base,
                segments: Vec::new(),
            })
            .ok_or_else(|| CellarError::UnsupportedPostInstallSyntax {
                message: format!("unsupported path expression: {raw}"),
            });
    };
    let (base, rest) = trimmed.split_at(split_index);
    let Some(base) = parse_path_base(base.trim()) else {
        return unsupported(&format!("unsupported path expression: {raw}"));
    };
    Ok(PathExpr {
        base,
        segments: parse_path_segments(rest)?,
    })
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

fn parse_path_segments(raw: &str) -> Result<Vec<String>, CellarError> {
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

fn parse_formula_ref(node: &Node<'_>) -> Result<Option<String>, CellarError> {
    let Some(call) = node.as_call_node() else {
        return Ok(None);
    };
    if call_name(&call)? != "[]" {
        return Ok(None);
    }
    let Some(receiver) = call.receiver() else {
        return Ok(None);
    };
    let Some(receiver) = receiver.as_constant_read_node() else {
        return Ok(None);
    };
    let receiver_name = receiver.name();
    if constant_name(&receiver_name)? != "Formula" {
        return Ok(None);
    }
    let arguments = call_args(&call);
    if arguments.len() != 1 {
        return unsupported("Formula[] expects one argument");
    }
    parse_string(&arguments[0])
}

fn parse_exist_condition<'pr>(
    node: &Node<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Option<PathExpr>, CellarError> {
    let Some(call) = node.as_call_node() else {
        return Ok(None);
    };
    if call_name(&call)? != "exist?" {
        return Ok(None);
    }
    let Some(receiver) = call.receiver() else {
        return Ok(None);
    };
    Ok(Some(parse_path_expr(
        &receiver,
        parsed,
        methods,
        helper_stack,
    )?))
}

fn predicate_is_os_runtime(node: &Node<'_>, method: &str) -> Result<bool, CellarError> {
    let Some(call) = node.as_call_node() else {
        return Ok(false);
    };
    if call_name(&call)? != method {
        return Ok(false);
    }
    let Some(receiver) = call.receiver() else {
        return Ok(false);
    };
    let Some(receiver) = receiver.as_constant_read_node() else {
        return Ok(false);
    };
    let receiver_name = receiver.name();
    Ok(constant_name(&receiver_name)? == "OS")
}

fn call_args<'pr>(call: &ruby_prism::CallNode<'pr>) -> Vec<Node<'pr>> {
    call.arguments()
        .map(|arguments| arguments.arguments().iter().collect())
        .unwrap_or_default()
}

fn call_name(call: &ruby_prism::CallNode<'_>) -> Result<String, CellarError> {
    let name = call.name();
    constant_name(&name)
}

fn constant_name(id: &ConstantId<'_>) -> Result<String, CellarError> {
    std::str::from_utf8(id.as_slice())
        .map(ToOwned::to_owned)
        .map_err(|error| CellarError::UnsupportedPostInstallSyntax {
            message: format!("invalid prism identifier utf-8: {error}"),
        })
}

fn parse_string(node: &Node<'_>) -> Result<Option<String>, CellarError> {
    if let Some(string) = node.as_string_node() {
        return String::from_utf8(string.unescaped().to_vec())
            .map(Some)
            .map_err(|error| CellarError::UnsupportedPostInstallSyntax {
                message: format!("invalid utf-8 string literal: {error}"),
            });
    }
    Ok(None)
}

fn is_force_true_keyword(node: &Node<'_>, parsed: &ParseResult<'_>) -> Result<bool, CellarError> {
    let source = node_source(parsed, node)?;
    Ok(source.trim() == "force: true")
}

fn node_source<'pr>(parsed: &ParseResult<'pr>, node: &Node<'pr>) -> Result<&'pr str, CellarError> {
    std::str::from_utf8(parsed.as_slice(&node.location())).map_err(|error| {
        CellarError::UnsupportedPostInstallSyntax {
            message: format!("invalid source utf-8: {error}"),
        }
    })
}

fn visit_calls<'pr, F>(node: &Node<'pr>, visit: &mut F) -> Result<(), CellarError>
where
    F: FnMut(&ruby_prism::CallNode<'pr>) -> Result<(), CellarError>,
{
    if let Some(call) = node.as_call_node() {
        visit(&call)?;
    }

    if let Some(program) = node.as_program_node() {
        for child in &program.statements().body() {
            visit_calls(&child, visit)?;
        }
    } else if let Some(statements) = node.as_statements_node() {
        for child in &statements.body() {
            visit_calls(&child, visit)?;
        }
    } else if let Some(def) = node.as_def_node() {
        if let Some(body) = def.body() {
            visit_calls(&body, visit)?;
        }
    } else if let Some(if_node) = node.as_if_node() {
        visit_calls(&if_node.predicate(), visit)?;
        if let Some(statements) = if_node.statements() {
            for child in &statements.body() {
                visit_calls(&child, visit)?;
            }
        }
        if let Some(subsequent) = if_node.subsequent() {
            visit_calls(&subsequent, visit)?;
        }
    } else if let Some(else_node) = node.as_else_node() {
        if let Some(statements) = else_node.statements() {
            for child in &statements.body() {
                visit_calls(&child, visit)?;
            }
        }
    } else if let Some(begin) = node.as_begin_node() {
        if let Some(statements) = begin.statements() {
            for child in &statements.body() {
                visit_calls(&child, visit)?;
            }
        }
    } else if let Some(parentheses) = node.as_parentheses_node()
        && let Some(body) = parentheses.body()
    {
        visit_calls(&body, visit)?;
    }

    Ok(())
}

fn append_segment(path: &PathExpr, segment: &str) -> PathExpr {
    let mut next = path.clone();
    next.segments.push(segment.to_owned());
    next
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
            Statement::RemoveIfExists(path) => {
                remove_path_if_exists(&context.resolve_path(path))?;
            }
            Statement::InstallSymlink { link_dir, target } => {
                let link_dir = context.resolve_path(link_dir);
                std::fs::create_dir_all(&link_dir)?;
                let target = context.resolve_path(target);
                let link_path = install_symlink_path(&link_dir, &target)?;
                remove_path_if_exists(&link_path)?;
                let link_target = relative_from_to(&link_dir, &target);
                std::os::unix::fs::symlink(link_target, link_path)?;
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

fn install_symlink_path(link_dir: &Path, target: &Path) -> Result<PathBuf, CellarError> {
    let Some(name) = target.file_name() else {
        return unsupported("install_symlink target must have file name");
    };
    Ok(link_dir.join(name))
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
