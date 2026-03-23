use std::collections::{BTreeMap, BTreeSet};

use ruby_prism::{CallNode, ConstantId, Node, ParseResult, Visit, parse as parse_ruby};

use crate::error::AnalysisError;

mod schema;

/// Controls which tier of DSL constructs the lowering accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoweringTier {
    /// Tier 1: static analysis only. Unknown formula attributes are rejected.
    Static,
    /// Tier 2: allow symbolic formula attributes (`name`, `version.major_minor`)
    /// that will be resolved at install time.
    WithAttributes,
}

/// Shared lowering context threaded through internal functions.
#[derive(Clone, Copy)]
struct LowerCtx<'a, 'pr> {
    parsed: &'a ParseResult<'pr>,
    methods: &'a BTreeMap<String, MethodDef<'pr>>,
    formula_version: &'a str,
    tier: LoweringTier,
}

/// Parsed `post_install` program — a sequence of lowered statements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// The lowered statement sequence.
    pub statements: Vec<Statement>,
}

/// A single lowered `post_install` operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    /// Create a directory tree.
    Mkpath(PathExpr),
    /// Copy a single file.
    Copy {
        /// Source path.
        from: PathExpr,
        /// Destination path.
        to: PathExpr,
    },
    /// Remove a path if it exists.
    RemoveIfExists(PathExpr),
    /// Create a relative symlink inside `link_dir` pointing to `target`.
    InstallSymlink {
        /// Directory that will contain the symlink.
        link_dir: PathExpr,
        /// Path the symlink will point to.
        target: PathExpr,
    },
    /// Execute a system command.
    System(Vec<Argument>),
    /// Conditional execution based on path state.
    IfPath {
        /// Path to check.
        condition: PathExpr,
        /// Kind of path check.
        kind: PathCondition,
        /// Statements to execute if the condition holds.
        then_branch: Vec<Self>,
    },
    /// Recursively copy a directory.
    RecursiveCopy {
        /// Source path.
        from: PathExpr,
        /// Destination parent path.
        to: PathExpr,
    },
    /// Copy children of a directory into another directory.
    CopyChildren {
        /// Source directory.
        from_dir: PathExpr,
        /// Destination directory.
        to_dir: PathExpr,
    },
    /// Force-create a symlink, removing any existing target.
    ForceSymlink {
        /// Path the symlink will point to.
        target: PathExpr,
        /// Path of the symlink itself.
        link: PathExpr,
    },
    /// Write content to a file.
    WriteFile {
        /// File path.
        path: PathExpr,
        /// File content parts (may include runtime substitutions).
        content: Vec<ContentPart>,
    },
    /// Remove entries in a directory matching a glob pattern.
    GlobRemove {
        /// Directory to scan.
        dir: PathExpr,
        /// Glob pattern.
        pattern: String,
    },
    /// Change permissions for entries in a directory matching a glob pattern.
    GlobChmod {
        /// Directory to scan.
        dir: PathExpr,
        /// Glob pattern.
        pattern: String,
        /// Unix permission mode.
        mode: u32,
    },
    /// Create symlinks for entries matching a glob pattern.
    GlobSymlink {
        /// Source directory to scan.
        source_dir: PathExpr,
        /// Glob pattern.
        pattern: String,
        /// Directory that will contain the symlinks.
        link_dir: PathExpr,
    },
    /// Install a file into a directory (moves `from` into `into_dir`).
    Install {
        /// Destination directory.
        into_dir: PathExpr,
        /// Source file path.
        from: PathExpr,
    },
    /// Move/rename a file or directory.
    Move {
        /// Source path.
        from: PathExpr,
        /// Destination path.
        to: PathExpr,
    },
    /// Move all children from one directory into another directory.
    MoveChildren {
        /// Source directory whose children will be moved.
        from_dir: PathExpr,
        /// Destination directory receiving the children.
        to_dir: PathExpr,
    },
    /// Change file permissions.
    Chmod {
        /// File path.
        path: PathExpr,
        /// Unix permission mode (e.g. `0o755`).
        mode: u32,
    },
    /// Mirror a source directory tree into a destination directory as symlinks.
    ///
    /// Walks the source tree recursively. For each entry:
    /// - Files and symlinks in source become symlinks in dest (via `install_symlink`)
    /// - Directories become real directories in dest (retaining existing real dirs)
    /// - Existing conflicting entries at destination are removed first
    /// - Entries matching `prune_names` are skipped entirely
    MirrorTree {
        /// Source directory to mirror.
        source: PathExpr,
        /// Destination directory for the mirror.
        dest: PathExpr,
        /// Basenames to skip (e.g. `".DS_Store"`).
        prune_names: Vec<String>,
    },
    /// Create symlinks in `link_dir` for each child of `source_dir`, appending a
    /// suffix to the link name.
    ChildrenSymlink {
        /// Directory whose children will be linked.
        source_dir: PathExpr,
        /// Directory that will contain the symlinks.
        link_dir: PathExpr,
        /// Suffix parts appended to each child's basename to form the link name.
        suffix: Vec<SegmentPart>,
    },
    /// Conditional execution based on an environment variable.
    ///
    /// If the named variable is set (non-empty), the `then_branch` is executed.
    /// If `negate` is true, executes when the variable is *not* set.
    IfEnv {
        /// Environment variable name.
        variable: String,
        /// Whether to negate the condition.
        negate: bool,
        /// Statements to execute when the condition holds.
        then_branch: Vec<Self>,
    },
    /// Set an environment variable for subsequent spawned commands.
    SetEnv {
        /// Environment variable name.
        variable: String,
        /// Environment variable value.
        value: Vec<ContentPart>,
    },
    /// Execute a process and capture its stdout as a string.
    ///
    /// Corresponds to Ruby's `Utils.safe_popen_read(path, args)`.
    /// The captured output is stored in a named variable accessible to
    /// subsequent statements via [`SegmentPart::CapturedOutput`].
    ProcessCapture {
        /// Variable name to store the captured output.
        variable: String,
        /// Command and arguments.
        command: Vec<Argument>,
    },
}

/// Path condition for [`Statement::IfPath`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathCondition {
    /// Path exists (file, directory, or symlink).
    Exists,
    /// Path does not exist.
    Missing,
    /// Path is a symlink.
    Symlink,
    /// Path exists and is not a symlink.
    ExistsAndNotSymlink,
}

/// A content fragment for [`Statement::WriteFile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentPart {
    /// Literal text.
    Literal(String),
    /// Substituted with the Homebrew prefix at runtime.
    HomebrewPrefix,
    /// Substituted with a segment part at runtime.
    Runtime(SegmentPart),
}

/// A command argument for [`Statement::System`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Argument {
    /// A path expression.
    Path(PathExpr),
    /// A literal string.
    String(String),
}

/// A symbolic path expression with a base and optional segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathExpr {
    /// The base path (e.g. `prefix`, `bin`, `HOMEBREW_PREFIX`).
    pub base: PathBase,
    /// Path segments joined after the base.
    pub segments: Vec<PathSegment>,
}

impl PathExpr {
    /// Creates a new path expression from a base and literal segment slices.
    #[must_use]
    pub fn new(base: PathBase, segments: &[&str]) -> Self {
        Self {
            base,
            segments: segments
                .iter()
                .map(|&s| PathSegment::Literal(s.to_owned()))
                .collect(),
        }
    }
}

/// A single segment in a [`PathExpr`].
///
/// Most segments are [`Literal`](PathSegment::Literal) strings known at
/// analysis time. [`Interpolated`](PathSegment::Interpolated) segments
/// contain parts that require runtime context (Tier 2) to resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    /// A segment whose value is fully known at analysis time.
    Literal(String),
    /// A segment composed of literal and runtime-resolved parts.
    Interpolated(Vec<SegmentPart>),
}

/// A fragment of an [`Interpolated`](PathSegment::Interpolated) path segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentPart {
    /// Literal text.
    Literal(String),
    /// The formula's `name` attribute, resolved at install time.
    FormulaName,
    /// The formula's `version.major_minor` value (e.g. `"3.12"` from `"3.12.2"`).
    VersionMajorMinor,
    /// The formula's `version.major` value (e.g. `"17"` from `"17.2"`).
    VersionMajor,
    /// The OS kernel version major (e.g. `"24"` on macOS Sequoia).
    KernelVersionMajor,
    /// The macOS version string (e.g. `"15.1"`).
    MacOSVersion,
    /// The CPU architecture string (e.g. `"arm64"`).
    CpuArch,
    /// A reference to a captured process output variable.
    CapturedOutput(String),
    /// The basename of a captured process output path.
    CapturedOutputBasename(String),
}

/// Base component of a [`PathExpr`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathBase {
    /// Keg root (`prefix`).
    Prefix,
    /// `prefix/bin`.
    Bin,
    /// `HOMEBREW_PREFIX/etc`.
    Etc,
    /// `HOMEBREW_PREFIX/etc/<formula>` via `Formula["name"].pkgetc`.
    FormulaPkgetc(String),
    /// `HOMEBREW_PREFIX/opt/<formula>/bin` via `Formula["name"].opt_bin`.
    FormulaOptBin(String),
    /// `HOMEBREW_PREFIX` (the Homebrew prefix root).
    HomebrewPrefix,
    /// `prefix/lib`.
    Lib,
    /// `prefix/libexec`.
    Libexec,
    /// `HOMEBREW_PREFIX/etc/<formula_name>`.
    Pkgetc,
    /// `prefix/share/<formula_name>`.
    Pkgshare,
    /// `prefix/sbin`.
    Sbin,
    /// `prefix/share`.
    Share,
    /// `HOMEBREW_PREFIX/var`.
    Var,
}

#[derive(Debug)]
struct MethodDef<'pr> {
    body: Option<Node<'pr>>,
    has_receiver: bool,
    has_parameters: bool,
}

/// Extracts the contents of `def post_install ... end`.
///
/// # Errors
///
/// Returns [`AnalysisError::UnsupportedPostInstallSyntax`] when the method is
/// missing or block boundaries cannot be matched.
pub fn extract_post_install_block(source: &str) -> Result<String, AnalysisError> {
    let parsed = parse_source(source)?;
    let methods = build_method_table(&parsed)?;
    let method =
        methods
            .get("post_install")
            .ok_or_else(|| AnalysisError::UnsupportedPostInstallSyntax {
                message: "missing def post_install block".to_owned(),
            })?;
    let body = method
        .body
        .as_ref()
        .ok_or_else(|| AnalysisError::UnsupportedPostInstallSyntax {
            message: "missing def post_install block".to_owned(),
        })?;

    node_source(&parsed, body).map(ToOwned::to_owned)
}

/// Validates that a `post_install` block can be parsed and lowered without executing it.
///
/// Use this to check whether a formula's `post_install` is supported before
/// performing destructive operations like unlinking the old keg during upgrade.
///
/// # Errors
///
/// Returns [`AnalysisError::UnsupportedPostInstallSyntax`] if the source contains
/// unsupported Ruby constructs that cannot be lowered.
pub fn validate_post_install(source: &str, formula_version: &str) -> Result<(), AnalysisError> {
    let _program = lower_post_install(source, formula_version)?;
    Ok(())
}

/// Parses and lowers a `post_install` block into an executable [`Program`].
///
/// This is Tier 1 (static analysis): unknown formula attributes like `name` or
/// `version.major_minor` are rejected.
///
/// # Errors
///
/// Returns [`AnalysisError::UnsupportedPostInstallSyntax`] if the source contains
/// unsupported Ruby constructs.
pub fn lower_post_install(source: &str, formula_version: &str) -> Result<Program, AnalysisError> {
    lower_post_install_inner(source, formula_version, LoweringTier::Static)
}

/// Parses and lowers a `post_install` block with Tier 2 attribute support.
///
/// Unlike [`lower_post_install`], this allows symbolic formula attributes
/// (`name`, `version.major_minor`) that will be resolved at install time by
/// the cellar's runtime context.
///
/// # Errors
///
/// Returns [`AnalysisError::UnsupportedPostInstallSyntax`] if the source contains
/// unsupported Ruby constructs beyond Tier 2 capabilities.
pub fn lower_post_install_tier2(
    source: &str,
    formula_version: &str,
) -> Result<Program, AnalysisError> {
    lower_post_install_inner(source, formula_version, LoweringTier::WithAttributes)
}

fn lower_post_install_inner(
    source: &str,
    formula_version: &str,
    tier: LoweringTier,
) -> Result<Program, AnalysisError> {
    let parsed = parse_source(source)?;
    let methods = build_method_table(&parsed)?;
    let ctx = LowerCtx {
        parsed: &parsed,
        methods: &methods,
        formula_version,
        tier,
    };
    let mut helper_stack = BTreeSet::new();
    let statements = lower_method("post_install", &ctx, &mut helper_stack)?;
    Ok(Program { statements })
}

fn parse_source(source: &str) -> Result<ParseResult<'_>, AnalysisError> {
    let parsed = parse_ruby(source.as_bytes());
    if let Some(error) = parsed.errors().next() {
        return unsupported(&format!("prism parse error: {}", error.message()));
    }
    Ok(parsed)
}

fn build_method_table<'pr>(
    parsed: &'pr ParseResult<'pr>,
) -> Result<BTreeMap<String, MethodDef<'pr>>, AnalysisError> {
    let mut methods = BTreeMap::new();
    collect_methods_from_node(&parsed.node(), &mut methods)?;
    Ok(methods)
}

fn collect_methods_from_node<'pr>(
    node: &Node<'pr>,
    methods: &mut BTreeMap<String, MethodDef<'pr>>,
) -> Result<(), AnalysisError> {
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

fn lower_method(
    name: &str,
    ctx: &LowerCtx<'_, '_>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, AnalysisError> {
    let method =
        ctx.methods
            .get(name)
            .ok_or_else(|| AnalysisError::UnsupportedPostInstallSyntax {
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
        if let Some(statements) = normalize_method_schema(body, ctx, helper_stack)? {
            return Ok(statements);
        }
        lower_body_node(body, ctx, helper_stack)
    })();

    helper_stack.remove(name);
    result
}

fn normalize_method_schema<'pr>(
    body: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Option<Vec<Statement>>, AnalysisError> {
    if let Some(stmts) = schema::match_gdk_pixbuf_loader_schema(body, ctx, helper_stack)? {
        return Ok(Some(stmts));
    }

    if let Some(stmts) = schema::match_postgresql_schemas(body, ctx, helper_stack)? {
        return Ok(Some(stmts));
    }

    if let Some(stmts) = schema::match_mysql_schema(body, ctx, helper_stack)? {
        return Ok(Some(stmts));
    }

    if let Some(stmts) = schema::match_llvm_clang_config_schema(body, ctx)? {
        return Ok(Some(stmts));
    }

    if schema::matches_ruby_bundler_cleanup_schema(body, ctx.methods, ctx.formula_version)? {
        return Ok(Some(schema::normalize_ruby_bundler_cleanup(
            ctx.formula_version,
        )));
    }

    if schema::matches_node_npm_propagation_schema(body, ctx.parsed)? {
        return Ok(Some(schema::normalize_node_npm_propagation()));
    }

    if let Some(stmts) = schema::match_python_site_packages_schema(body, ctx)? {
        return Ok(Some(stmts));
    }

    if let Some(stmts) = schema::match_php_pear_schema(body, ctx)? {
        return Ok(Some(stmts));
    }

    if schema::matches_shared_mime_info_schema(body, ctx.parsed)? {
        return Ok(Some(schema::normalize_shared_mime_info()));
    }

    if schema::matches_bundle_bootstrap_schema(body, ctx, helper_stack)? {
        return Ok(Some(vec![
            Statement::Mkpath(PathExpr {
                base: PathBase::Pkgetc,
                segments: Vec::new(),
            }),
            Statement::Copy {
                from: PathExpr {
                    base: PathBase::Pkgshare,
                    segments: vec![PathSegment::Literal("cacert.pem".to_owned())],
                },
                to: PathExpr {
                    base: PathBase::Pkgetc,
                    segments: vec![PathSegment::Literal("cert.pem".to_owned())],
                },
            },
        ]));
    }

    if let Some((link_dir, target)) = schema::detect_cert_symlink_schema(body, ctx, helper_stack)? {
        return Ok(Some(vec![
            Statement::RemoveIfExists(append_segment(&link_dir, "cert.pem")),
            Statement::InstallSymlink { link_dir, target },
        ]));
    }

    Ok(None)
}

fn lower_body_node<'pr>(
    body: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, AnalysisError> {
    let mut statements = Vec::new();
    for child in body_statements(body)? {
        statements.extend(lower_statement(&child, ctx, helper_stack)?);
    }
    Ok(statements)
}

fn body_statements<'pr>(body: &Node<'pr>) -> Result<Vec<Node<'pr>>, AnalysisError> {
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
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, AnalysisError> {
    if let Some(if_node) = node.as_if_node() {
        return lower_if_statement(&if_node, ctx, helper_stack);
    }

    if let Some(call) = node.as_call_node() {
        return lower_call_statement(&call, ctx, helper_stack);
    }

    if let Some(unless_node) = node.as_unless_node() {
        return lower_unless_statement(&unless_node, ctx, helper_stack);
    }

    // Tier 2: local variable assignment with Utils.safe_popen_read
    if ctx.tier == LoweringTier::WithAttributes
        && let Some(assign) = node.as_local_variable_write_node()
    {
        return lower_local_assign(&assign, ctx, helper_stack);
    }

    unsupported(&format!(
        "unsupported post_install statement: {}",
        node_source(ctx.parsed, node)?
    ))
}

fn lower_local_assign<'pr>(
    assign: &ruby_prism::LocalVariableWriteNode<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, AnalysisError> {
    let var_name = constant_name(&assign.name())?;
    let value = assign.value();

    // Check for Utils.safe_popen_read(...)
    if let Some(call) = value.as_call_node()
        && is_safe_popen_read(&call)?
    {
        let arguments = call_args(&call);
        if arguments.is_empty() {
            return unsupported("Utils.safe_popen_read expects at least one argument");
        }
        let command = arguments
            .iter()
            .map(|arg| parse_argument(arg, ctx, helper_stack))
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(vec![Statement::ProcessCapture {
            variable: var_name,
            command,
        }]);
    }

    unsupported(&format!(
        "unsupported local variable assignment: {}",
        node_source(ctx.parsed, &assign.as_node())?
    ))
}

/// Checks if a call node is `Utils.safe_popen_read(...)`.
fn is_safe_popen_read(call: &ruby_prism::CallNode<'_>) -> Result<bool, AnalysisError> {
    if call_name(call)? != "safe_popen_read" {
        return Ok(false);
    }
    let Some(receiver) = call.receiver() else {
        return Ok(false);
    };
    let Some(constant) = receiver.as_constant_read_node() else {
        return Ok(false);
    };
    Ok(constant_name(&constant.name())? == "Utils")
}

fn lower_if_statement<'pr>(
    if_node: &ruby_prism::IfNode<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, AnalysisError> {
    if predicate_is_os_runtime(&if_node.predicate(), "mac?")? {
        return lower_statements_node_opt(if_node.statements(), ctx, helper_stack);
    }

    if predicate_is_os_runtime(&if_node.predicate(), "linux?")? {
        return lower_else_branch(if_node.subsequent(), ctx, helper_stack);
    }

    if let Some(condition) = parse_exist_condition(&if_node.predicate(), ctx, helper_stack)? {
        let then_branch = lower_statements_node_opt(if_node.statements(), ctx, helper_stack)?;
        if if_node.subsequent().is_none()
            && then_branch.len() == 1
            && matches!(then_branch.first(), Some(Statement::RemoveIfExists(path)) if *path == condition)
        {
            return Ok(then_branch);
        }
        if if_node.subsequent().is_some() {
            return unsupported("unsupported else branch for path existence condition");
        }
        return Ok(vec![Statement::IfPath {
            condition,
            kind: PathCondition::Exists,
            then_branch,
        }]);
    }

    unsupported(&format!(
        "unsupported post_install conditional: {}",
        node_source(ctx.parsed, &if_node.predicate())?
    ))
}

fn lower_unless_statement<'pr>(
    unless_node: &ruby_prism::UnlessNode<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, AnalysisError> {
    // `unless OS.mac?` — on macOS OS.mac? is TRUE → unless body never executes → skip
    if predicate_is_os_runtime(&unless_node.predicate(), "mac?")? {
        return Ok(vec![]);
    }
    // `unless OS.linux?` — on macOS OS.linux? is FALSE → unless body always executes
    if predicate_is_os_runtime(&unless_node.predicate(), "linux?")? {
        return lower_statements_node_opt(unless_node.statements(), ctx, helper_stack);
    }
    unsupported(&format!(
        "unsupported unless condition: {}",
        node_source(ctx.parsed, &unless_node.predicate())?
    ))
}

fn lower_else_branch<'pr>(
    subsequent: Option<Node<'pr>>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, AnalysisError> {
    let Some(subsequent) = subsequent else {
        return Ok(Vec::new());
    };
    if let Some(else_node) = subsequent.as_else_node() {
        return lower_statements_node_opt(else_node.statements(), ctx, helper_stack);
    }
    if let Some(if_node) = subsequent.as_if_node() {
        return lower_if_statement(&if_node, ctx, helper_stack);
    }
    unsupported("unsupported runtime else branch")
}

fn lower_statements_node_opt<'pr>(
    statements: Option<ruby_prism::StatementsNode<'pr>>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, AnalysisError> {
    let Some(statements) = statements else {
        return Ok(Vec::new());
    };
    let mut lowered = Vec::new();
    for child in &statements.body() {
        lowered.extend(lower_statement(&child, ctx, helper_stack)?);
    }
    Ok(lowered)
}

fn lower_call_statement<'pr>(
    call: &ruby_prism::CallNode<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Vec<Statement>, AnalysisError> {
    let name = call_name(call)?;
    if let Some(receiver) = call.receiver()
        && let Some(constant) = receiver.as_constant_read_node()
        && constant_name(&constant.name())? == "ENV"
    {
        return lower_env_call(call);
    }

    if let Some(receiver) = call.receiver() {
        let receiver = parse_path_expr(&receiver, ctx, helper_stack)?;
        return match name.as_str() {
            "mkpath" => Ok(vec![Statement::Mkpath(receiver)]),
            "install_symlink" => {
                let arguments = call_args(call);
                if arguments.is_empty() {
                    return unsupported("install_symlink expects at least one argument");
                }
                arguments
                    .iter()
                    .map(|arg| {
                        Ok(Statement::InstallSymlink {
                            link_dir: receiver.clone(),
                            target: parse_path_expr(arg, ctx, helper_stack)?,
                        })
                    })
                    .collect()
            }
            "atomic_write" => {
                let arguments = call_args(call);
                if arguments.len() != 1 {
                    return unsupported("atomic_write expects exactly one argument");
                }
                let content = parse_content_parts(&arguments[0])?;
                Ok(vec![Statement::WriteFile {
                    path: receiver,
                    content,
                }])
            }
            "install" => {
                let arguments = call_args(call);
                if arguments.len() != 1 {
                    return unsupported("install expects exactly one argument");
                }
                Ok(vec![Statement::Install {
                    into_dir: receiver,
                    from: parse_path_expr(&arguments[0], ctx, helper_stack)?,
                }])
            }
            "chmod" => {
                let arguments = call_args(call);
                if arguments.len() != 1 {
                    return unsupported("chmod expects exactly one argument");
                }
                let mode = parse_integer_mode(&arguments[0])?;
                Ok(vec![Statement::Chmod {
                    path: receiver,
                    mode,
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
                from: parse_path_expr(&arguments[0], ctx, helper_stack)?,
                to: parse_path_expr(&arguments[1], ctx, helper_stack)?,
            }])
        }
        "system" | "quiet_system" => {
            let arguments = call_args(call);
            if arguments.is_empty() {
                return unsupported("system expects at least one argument");
            }
            let arguments = arguments
                .iter()
                .map(|argument| parse_argument(argument, ctx, helper_stack))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(vec![Statement::System(arguments)])
        }
        "rm" => {
            let arguments = call_args(call);
            if arguments.is_empty() || arguments.len() > 2 {
                return unsupported("rm expects one path argument");
            }
            if arguments.len() == 2 && !is_force_true_keyword(&arguments[1], ctx.parsed)? {
                return unsupported("rm only supports force: true keyword");
            }
            Ok(vec![Statement::RemoveIfExists(parse_path_expr(
                &arguments[0],
                ctx,
                helper_stack,
            )?)])
        }
        "mkdir_p" => {
            let arguments = call_args(call);
            if arguments.len() != 1 {
                return unsupported("mkdir_p expects exactly one argument");
            }
            Ok(vec![Statement::Mkpath(parse_path_expr(
                &arguments[0],
                ctx,
                helper_stack,
            )?)])
        }
        // Homebrew logging helpers — purely informational, safe to ignore.
        "ohai" | "opoo" | "odebug" => Ok(vec![]),
        helper if ctx.methods.contains_key(helper) => lower_method(helper, ctx, helper_stack),
        _ => unsupported(&format!("unsupported post_install statement: {name}")),
    }
}

fn lower_env_call(call: &ruby_prism::CallNode<'_>) -> Result<Vec<Statement>, AnalysisError> {
    match call_name(call)?.as_str() {
        "[]=" => {
            let arguments = call_args(call);
            if arguments.len() != 2 {
                return unsupported("ENV[]= expects exactly two arguments");
            }
            let Some(variable) = parse_string(&arguments[0])? else {
                return unsupported("ENV[]= variable must be a string literal");
            };
            Ok(vec![Statement::SetEnv {
                variable,
                value: parse_content_parts(&arguments[1])?,
            }])
        }
        _ => unsupported("unsupported ENV call"),
    }
}

fn parse_argument<'pr>(
    node: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Argument, AnalysisError> {
    if let Some(value) = parse_string(node)? {
        return Ok(Argument::String(value));
    }
    Ok(Argument::Path(parse_path_expr(node, ctx, helper_stack)?))
}

fn parse_path_expr<'pr>(
    node: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<PathExpr, AnalysisError> {
    if let Ok(path) = parse_path_expr_ast(node, ctx, helper_stack) {
        return Ok(path);
    }
    parse_path_expr_text(node_source(ctx.parsed, node)?)
}

fn parse_path_expr_ast<'pr>(
    node: &Node<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<PathExpr, AnalysisError> {
    if let Some(parentheses) = node.as_parentheses_node()
        && let Some(body) = parentheses.body()
    {
        // ruby-prism wraps parenthesized expressions in StatementsNode;
        // unwrap to the single inner expression when possible.
        if let Some(statements) = body.as_statements_node() {
            let children: Vec<_> = statements.body().iter().collect();
            if children.len() == 1 {
                return parse_path_expr_ast(&children[0], ctx, helper_stack);
            }
        }
        return parse_path_expr_ast(&body, ctx, helper_stack);
    }

    if let Some(constant) = node.as_constant_read_node() {
        let name = constant.name();
        let name = constant_name(&name)?;
        if name == "HOMEBREW_PREFIX" {
            return Ok(PathExpr {
                base: PathBase::HomebrewPrefix,
                segments: Vec::new(),
            });
        }
        return unsupported(&format!("unsupported constant in path expression: {name}"));
    }

    if let Some(interp) = node.as_interpolated_string_node() {
        return parse_interpolated_path_expr_ast(&interp, ctx, helper_stack);
    }

    let Some(call) = node.as_call_node() else {
        return unsupported(&format!(
            "unsupported path expression: {}",
            node_source(ctx.parsed, node)?
        ));
    };
    let name = call_name(&call)?;
    if name == "/" {
        let Some(receiver) = call.receiver() else {
            return unsupported("path join requires receiver");
        };
        let mut path = parse_path_expr_ast(&receiver, ctx, helper_stack)?;
        let arguments = call_args(&call);
        if arguments.len() != 1 {
            return unsupported("path join expects exactly one segment");
        }
        // Try string literal first.
        if let Some(segment) = parse_string(&arguments[0])? {
            path.segments.extend(
                segment
                    .split('/')
                    .filter(|part| !part.is_empty())
                    .map(|part| {
                        validate_path_segment(part)
                            .map(|value| PathSegment::Literal(value.to_owned()))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );
            return Ok(path);
        }
        // Tier 2: allow formula attributes as path join segments.
        if ctx.tier == LoweringTier::WithAttributes {
            let tier2_segments = parse_tier2_segments(&arguments[0])?;
            if !tier2_segments.is_empty() {
                path.segments.extend(tier2_segments);
                return Ok(path);
            }
        }
        return unsupported("path join segment must be string literal");
    }

    if name == "parent" && call.arguments().is_none() {
        let Some(receiver) = call.receiver() else {
            return unsupported("parent requires receiver");
        };
        let mut path = parse_path_expr_ast(&receiver, ctx, helper_stack)?;
        if path.segments.pop().is_some() {
            return Ok(path);
        }
        return unsupported("parent requires at least one path segment");
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

    if name == "opt_bin"
        && let Some(receiver) = call.receiver()
        && let Some(formula) = parse_formula_ref(&receiver)?
    {
        return Ok(PathExpr {
            base: PathBase::FormulaOptBin(formula),
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
        return parse_helper_path_expr(&name, ctx, helper_stack);
    }

    unsupported(&format!(
        "unsupported path expression: {}",
        node_source(ctx.parsed, node)?
    ))
}

fn parse_interpolated_path_expr_ast<'pr>(
    interp: &ruby_prism::InterpolatedStringNode<'pr>,
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<PathExpr, AnalysisError> {
    let parts: Vec<_> = interp.parts().iter().collect();
    let Some(first) = parts.first() else {
        return unsupported("empty interpolated path expression");
    };
    let Some(embedded) = first.as_embedded_statements_node() else {
        return unsupported(&format!(
            "interpolated path must start with embedded expression: {}",
            node_source(ctx.parsed, first)?
        ));
    };
    let stmts =
        embedded
            .statements()
            .ok_or_else(|| AnalysisError::UnsupportedPostInstallSyntax {
                message: "empty embedded expression in interpolated path".to_owned(),
            })?;
    let body: Vec<_> = stmts.body().iter().collect();
    if body.len() != 1 {
        return unsupported("embedded path expression must be a single expression");
    }
    let mut path = parse_path_expr_ast(&body[0], ctx, helper_stack)?;
    for part in &parts[1..] {
        let Some(string) = part.as_string_node() else {
            return unsupported(&format!(
                "interpolated path segment must be string literal: {}",
                node_source(ctx.parsed, part)?
            ));
        };
        let segment_str = String::from_utf8(string.unescaped().to_vec()).map_err(|error| {
            AnalysisError::UnsupportedPostInstallSyntax {
                message: format!("invalid utf-8 in interpolated path: {error}"),
            }
        })?;
        for segment in segment_str.split('/').filter(|s| !s.is_empty()) {
            path.segments.push(PathSegment::Literal(
                validate_path_segment(segment)?.to_owned(),
            ));
        }
    }
    Ok(path)
}

fn parse_helper_path_expr(
    name: &str,
    ctx: &LowerCtx<'_, '_>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<PathExpr, AnalysisError> {
    let method =
        ctx.methods
            .get(name)
            .ok_or_else(|| AnalysisError::UnsupportedPostInstallSyntax {
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
        parse_path_expr(&statements[0], ctx, helper_stack)
    })();
    helper_stack.remove(&format!("path:{name}"));
    result
}

/// Recognizes a call node as a known formula attribute and returns the
/// corresponding [`SegmentPart`], or `None` if unrecognized.
///
/// Recognized patterns:
/// - Receiverless, no-arg `name` → [`SegmentPart::FormulaName`]
/// - `version.major_minor` → [`SegmentPart::VersionMajorMinor`]
fn recognize_formula_attribute(
    call: &ruby_prism::CallNode<'_>,
) -> Result<Option<SegmentPart>, AnalysisError> {
    let name = call_name(call)?;
    if name == "name" && call.receiver().is_none() && call.arguments().is_none() {
        return Ok(Some(SegmentPart::FormulaName));
    }
    if name == "major_minor"
        && call.arguments().is_none()
        && let Some(receiver) = call.receiver()
        && let Some(version_call) = receiver.as_call_node()
        && call_name(&version_call)? == "version"
        && version_call.receiver().is_none()
        && version_call.arguments().is_none()
    {
        return Ok(Some(SegmentPart::VersionMajorMinor));
    }
    Ok(None)
}

/// Attempts to parse a Tier 2 formula attribute as path segments.
///
/// Recognizes:
/// - `name` → `[Interpolated([FormulaName])]`
/// - `version.major_minor` → `[Interpolated([VersionMajorMinor])]`
/// - Interpolated strings containing these attributes (may produce multiple
///   segments when the string contains `/`)
fn parse_tier2_segments(node: &Node<'_>) -> Result<Vec<PathSegment>, AnalysisError> {
    if let Some(call) = node.as_call_node()
        && let Some(part) = recognize_formula_attribute(&call)?
    {
        return Ok(vec![PathSegment::Interpolated(vec![part])]);
    }

    if let Some(interp) = node.as_interpolated_string_node() {
        return parse_tier2_interpolated_segments(&interp);
    }

    Ok(Vec::new())
}

/// Parses an interpolated string as Tier 2 path segments.
///
/// The string may contain literal text and embedded formula attributes
/// (`name`, `version.major_minor`). `/` in literal parts splits the result
/// into multiple path segments.
fn parse_tier2_interpolated_segments(
    interp: &ruby_prism::InterpolatedStringNode<'_>,
) -> Result<Vec<PathSegment>, AnalysisError> {
    let parts: Vec<_> = interp.parts().iter().collect();
    if parts.is_empty() {
        return Ok(Vec::new());
    }

    // Collect all parts as SegmentParts first, then split on `/` boundaries.
    let mut flat_parts: Vec<SegmentPart> = Vec::new();

    for part in &parts {
        if let Some(string) = part.as_string_node() {
            let text = String::from_utf8(string.unescaped().to_vec()).map_err(|error| {
                AnalysisError::UnsupportedPostInstallSyntax {
                    message: format!("invalid utf-8 in interpolated segment: {error}"),
                }
            })?;
            if !text.is_empty() {
                flat_parts.push(SegmentPart::Literal(text));
            }
        } else if let Some(embedded) = part.as_embedded_statements_node() {
            let stmts = embedded.statements().ok_or_else(|| {
                AnalysisError::UnsupportedPostInstallSyntax {
                    message: "empty embedded expression in Tier 2 segment".to_owned(),
                }
            })?;
            let body: Vec<_> = stmts.body().iter().collect();
            if body.len() != 1 {
                return Ok(Vec::new());
            }
            let Some(call) = body[0].as_call_node() else {
                return Ok(Vec::new());
            };
            let Some(part) = recognize_formula_attribute(&call)? else {
                return Ok(Vec::new());
            };
            flat_parts.push(part);
        } else {
            return Ok(Vec::new());
        }
    }

    if flat_parts.is_empty() {
        return Ok(Vec::new());
    }

    // Split on `/` in literal parts to produce multiple path segments.
    split_segment_parts_on_slash(flat_parts)
}

/// Splits a flat list of [`SegmentPart`]s into multiple [`PathSegment`]s
/// wherever a `/` appears in a [`SegmentPart::Literal`].
fn split_segment_parts_on_slash(
    parts: Vec<SegmentPart>,
) -> Result<Vec<PathSegment>, AnalysisError> {
    let mut segments: Vec<PathSegment> = Vec::new();
    let mut current: Vec<SegmentPart> = Vec::new();

    for part in parts {
        match part {
            SegmentPart::Literal(ref text) if text.contains('/') => {
                let pieces: Vec<&str> = text.split('/').collect();
                for (i, piece) in pieces.iter().enumerate() {
                    if i > 0 {
                        // Flush current accumulator as a segment.
                        if !current.is_empty() {
                            segments.push(make_path_segment(std::mem::take(&mut current)));
                        }
                    }
                    if !piece.is_empty() {
                        current.push(SegmentPart::Literal((*piece).to_owned()));
                    }
                }
            }
            other => current.push(other),
        }
    }

    if !current.is_empty() {
        segments.push(make_path_segment(current));
    }

    // Validate: each segment must produce a non-empty result.
    for segment in &segments {
        match segment {
            PathSegment::Literal(s) => {
                validate_path_segment(s)?;
            }
            PathSegment::Interpolated(parts) => {
                if parts.is_empty() {
                    return unsupported("empty interpolated path segment");
                }
            }
        }
    }

    Ok(segments)
}

/// Converts a list of [`SegmentPart`]s into a single [`PathSegment`].
fn make_path_segment(parts: Vec<SegmentPart>) -> PathSegment {
    if parts.len() == 1
        && let SegmentPart::Literal(s) = &parts[0]
    {
        return PathSegment::Literal(s.clone());
    }
    PathSegment::Interpolated(parts)
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
        "HOMEBREW_PREFIX" => Some(PathBase::HomebrewPrefix),
        _ => None,
    }
}

fn parse_path_expr_text(raw: &str) -> Result<PathExpr, AnalysisError> {
    let trimmed = raw.trim().trim_start_matches('(').trim_end_matches(')');
    if let Some(path) = parse_interpolated_homebrew_prefix_path(trimmed)? {
        return Ok(path);
    }
    let Some(split_index) = find_path_split(trimmed) else {
        return parse_path_base(trimmed)
            .map(|base| PathExpr {
                base,
                segments: Vec::new(),
            })
            .ok_or_else(|| AnalysisError::UnsupportedPostInstallSyntax {
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

fn parse_interpolated_homebrew_prefix_path(raw: &str) -> Result<Option<PathExpr>, AnalysisError> {
    let Some(inner) = raw.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else {
        return Ok(None);
    };
    let Some(rest) = inner.strip_prefix("#{HOMEBREW_PREFIX}") else {
        return Ok(None);
    };
    let segments = rest
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| validate_path_segment(segment).map(|s| PathSegment::Literal(s.to_owned())))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(PathExpr {
        base: PathBase::HomebrewPrefix,
        segments,
    }))
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

fn parse_path_segments(raw: &str) -> Result<Vec<PathSegment>, AnalysisError> {
    let mut segments = Vec::new();
    let mut remainder = raw.trim();
    while !remainder.is_empty() {
        let without_slash = remainder.strip_prefix('/').ok_or_else(|| {
            AnalysisError::UnsupportedPostInstallSyntax {
                message: format!("unsupported path segment: {remainder}"),
            }
        })?;
        let Some(stripped) = without_slash.strip_prefix('"') else {
            return unsupported(&format!("unsupported path segment: {remainder}"));
        };
        let end =
            stripped
                .find('"')
                .ok_or_else(|| AnalysisError::UnsupportedPostInstallSyntax {
                    message: format!("unterminated string literal in path: {remainder}"),
                })?;
        segments.push(PathSegment::Literal(
            validate_path_segment(&stripped[..end])?.to_owned(),
        ));
        remainder = stripped[end + 1..].trim();
    }
    Ok(segments)
}

fn validate_path_segment(segment: &str) -> Result<&str, AnalysisError> {
    if segment.is_empty() || segment == "." || segment == ".." || segment.contains("#{") {
        return Err(AnalysisError::UnsupportedPostInstallSyntax {
            message: format!("unsupported path segment: {segment}"),
        });
    }
    Ok(segment)
}

fn parse_formula_ref(node: &Node<'_>) -> Result<Option<String>, AnalysisError> {
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
    ctx: &LowerCtx<'_, 'pr>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Option<PathExpr>, AnalysisError> {
    let Some(call) = node.as_call_node() else {
        return Ok(None);
    };
    if call_name(&call)? != "exist?" {
        return Ok(None);
    }
    let Some(receiver) = call.receiver() else {
        return Ok(None);
    };
    Ok(Some(parse_path_expr(&receiver, ctx, helper_stack)?))
}

fn predicate_is_os_runtime(node: &Node<'_>, method: &str) -> Result<bool, AnalysisError> {
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

fn call_name(call: &ruby_prism::CallNode<'_>) -> Result<String, AnalysisError> {
    let name = call.name();
    constant_name(&name)
}

fn constant_name(id: &ConstantId<'_>) -> Result<String, AnalysisError> {
    std::str::from_utf8(id.as_slice())
        .map(ToOwned::to_owned)
        .map_err(|error| AnalysisError::UnsupportedPostInstallSyntax {
            message: format!("invalid prism identifier utf-8: {error}"),
        })
}

fn parse_string(node: &Node<'_>) -> Result<Option<String>, AnalysisError> {
    if let Some(string) = node.as_string_node() {
        return String::from_utf8(string.unescaped().to_vec())
            .map(Some)
            .map_err(|error| AnalysisError::UnsupportedPostInstallSyntax {
                message: format!("invalid utf-8 string literal: {error}"),
            });
    }
    Ok(None)
}

fn parse_integer_mode(node: &Node<'_>) -> Result<u32, AnalysisError> {
    let integer =
        node.as_integer_node()
            .ok_or_else(|| AnalysisError::UnsupportedPostInstallSyntax {
                message: "chmod expects an integer argument".to_owned(),
            })?;
    let value: i32 =
        integer
            .value()
            .try_into()
            .map_err(|()| AnalysisError::UnsupportedPostInstallSyntax {
                message: "chmod mode too large for i32".to_owned(),
            })?;
    u32::try_from(value).map_err(|_err| AnalysisError::UnsupportedPostInstallSyntax {
        message: "chmod mode must be non-negative".to_owned(),
    })
}

/// Parses the argument to `atomic_write` into a [`Vec<ContentPart>`].
///
/// Accepts plain string literals and interpolated strings where the only
/// interpolation is `#{HOMEBREW_PREFIX}`.
fn parse_content_parts(node: &Node<'_>) -> Result<Vec<ContentPart>, AnalysisError> {
    if let Some(value) = parse_string(node)? {
        return Ok(vec![ContentPart::Literal(value)]);
    }
    if let Some(interp) = node.as_interpolated_string_node() {
        let mut parts = Vec::new();
        for part in &interp.parts() {
            if let Some(string) = part.as_string_node() {
                let value = String::from_utf8(string.unescaped().to_vec()).map_err(|error| {
                    AnalysisError::UnsupportedPostInstallSyntax {
                        message: format!("invalid utf-8 in string content: {error}"),
                    }
                })?;
                if !value.is_empty() {
                    parts.push(ContentPart::Literal(value));
                }
            } else if let Some(embedded) = part.as_embedded_statements_node() {
                let stmts = embedded.statements().ok_or_else(|| {
                    AnalysisError::UnsupportedPostInstallSyntax {
                        message: "empty interpolation in atomic_write argument".to_owned(),
                    }
                })?;
                let body: Vec<_> = stmts.body().iter().collect();
                if body.len() != 1 {
                    return unsupported("atomic_write interpolation must be a single expression");
                }
                let Some(constant) = body[0].as_constant_read_node() else {
                    return unsupported("atomic_write only supports HOMEBREW_PREFIX interpolation");
                };
                if constant_name(&constant.name())? != "HOMEBREW_PREFIX" {
                    return unsupported("atomic_write only supports HOMEBREW_PREFIX interpolation");
                }
                parts.push(ContentPart::HomebrewPrefix);
            } else {
                return unsupported("unsupported interpolation part in atomic_write argument");
            }
        }
        return Ok(parts);
    }
    unsupported("atomic_write argument must be a string literal")
}

fn is_force_true_keyword(node: &Node<'_>, parsed: &ParseResult<'_>) -> Result<bool, AnalysisError> {
    let source = node_source(parsed, node)?;
    Ok(source.trim() == "force: true")
}

fn node_source<'pr>(
    parsed: &ParseResult<'pr>,
    node: &Node<'pr>,
) -> Result<&'pr str, AnalysisError> {
    std::str::from_utf8(parsed.as_slice(&node.location())).map_err(|error| {
        AnalysisError::UnsupportedPostInstallSyntax {
            message: format!("invalid source utf-8: {error}"),
        }
    })
}

fn visit_calls<'pr, F>(node: &Node<'pr>, visit: &mut F) -> Result<(), AnalysisError>
where
    F: FnMut(&ruby_prism::CallNode<'pr>) -> Result<(), AnalysisError>,
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
    next.segments.push(PathSegment::Literal(segment.to_owned()));
    next
}

fn unsupported<T>(message: &str) -> Result<T, AnalysisError> {
    Err(AnalysisError::UnsupportedPostInstallSyntax {
        message: message.to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Feature census
// ---------------------------------------------------------------------------

/// Feature census for a formula `post_install` block.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostInstallFeatures {
    // --- Receiverless calls ---
    /// Uses `system` or `quiet_system`.
    pub system: bool,
    /// Uses `cp`.
    pub cp: bool,
    /// Uses `rm`.
    pub rm: bool,
    /// Uses `mkdir_p`.
    pub mkdir_p: bool,
    /// Uses `ohai`, `opoo`, or `odebug`.
    pub log: bool,

    // --- Receiver calls ---
    /// Uses `.mkpath`.
    pub mkpath: bool,
    /// Uses `.install_symlink`.
    pub install_symlink: bool,
    /// Uses `.atomic_write`.
    pub atomic_write: bool,
    /// Uses `.chmod`.
    pub chmod: bool,
    /// Uses `.install`.
    pub install: bool,

    // --- ENV ---
    /// Uses `ENV[]=`.
    pub env: bool,

    // --- Control flow ---
    /// Uses `if` or `unless` with `OS.mac?` or `OS.linux?`.
    pub os_condition: bool,
    /// Uses `if` with `.exist?`, `.symlink?`, or `.directory?`.
    pub path_condition: bool,

    // --- Path bases ---
    /// Uses `prefix`.
    pub prefix: bool,
    /// Uses `bin`.
    pub bin: bool,
    /// Uses `etc`.
    pub etc: bool,
    /// Uses `lib`.
    pub lib: bool,
    /// Uses `libexec`.
    pub libexec: bool,
    /// Uses `pkgetc`.
    pub pkgetc: bool,
    /// Uses `pkgshare`.
    pub pkgshare: bool,
    /// Uses `sbin`.
    pub sbin: bool,
    /// Uses `share`.
    pub share: bool,
    /// Uses `var`.
    pub var: bool,
    /// Uses `HOMEBREW_PREFIX`.
    pub homebrew_prefix: bool,

    // --- Other ---
    /// Calls custom helper methods defined in the formula class.
    pub helper_methods: bool,
    /// Uses `name` attribute.
    pub formula_name: bool,
    /// Uses `version.major_minor` or `version.major`.
    pub formula_version: bool,
    /// Uses `Utils.safe_popen_read`.
    pub process_capture: bool,
    /// Uses `Formula["..."]` references.
    pub formula_ref: bool,
}

impl PostInstallFeatures {
    /// Returns the enabled feature names in a stable order.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        macro_rules! push_if {
            ($field:ident, $name:literal) => {
                if self.$field {
                    names.push($name);
                }
            };
        }
        push_if!(system, "system");
        push_if!(cp, "cp");
        push_if!(rm, "rm");
        push_if!(mkdir_p, "mkdir_p");
        push_if!(log, "log");
        push_if!(mkpath, "mkpath");
        push_if!(install_symlink, "install_symlink");
        push_if!(atomic_write, "atomic_write");
        push_if!(chmod, "chmod");
        push_if!(install, "install");
        push_if!(env, "ENV");
        push_if!(os_condition, "os_condition");
        push_if!(path_condition, "path_condition");
        push_if!(prefix, "prefix");
        push_if!(bin, "bin");
        push_if!(etc, "etc");
        push_if!(lib, "lib");
        push_if!(libexec, "libexec");
        push_if!(pkgetc, "pkgetc");
        push_if!(pkgshare, "pkgshare");
        push_if!(sbin, "sbin");
        push_if!(share, "share");
        push_if!(var, "var");
        push_if!(homebrew_prefix, "HOMEBREW_PREFIX");
        push_if!(helper_methods, "helper_methods");
        push_if!(formula_name, "formula_name");
        push_if!(formula_version, "formula_version");
        push_if!(process_capture, "process_capture");
        push_if!(formula_ref, "formula_ref");
        names
    }
}

// ---------------------------------------------------------------------------
// Feature collector visitor
// ---------------------------------------------------------------------------

struct PostInstallFeatureCollector<'a> {
    features: PostInstallFeatures,
    user_methods: &'a BTreeSet<&'a str>,
}

/// Returns `true` when the call node's receiver is a constant read matching
/// `expected` (e.g. `"ENV"`, `"Utils"`, `"Formula"`).
fn receiver_is_constant(node: &CallNode<'_>, expected: &str) -> bool {
    let Some(receiver) = node.receiver() else {
        return false;
    };
    let Some(constant) = receiver.as_constant_read_node() else {
        return false;
    };
    matches!(constant_name(&constant.name()).as_deref(), Ok(n) if n == expected)
}

/// Returns `true` when the receiver is a call to `"version"` (no receiver, no
/// args), i.e. `version.major_minor` or `version.major`.
fn receiver_is_version_call(node: &CallNode<'_>) -> bool {
    let Some(receiver) = node.receiver() else {
        return false;
    };
    let Some(call) = receiver.as_call_node() else {
        return false;
    };
    matches!(call_name(&call).as_deref(), Ok("version"))
}

impl PostInstallFeatureCollector<'_> {
    fn visit_receiverless(&mut self, name: &str, has_args: bool) {
        match name {
            "system" | "quiet_system" => self.features.system = true,
            "cp" => self.features.cp = true,
            "rm" => self.features.rm = true,
            "mkdir_p" => self.features.mkdir_p = true,
            "ohai" | "opoo" | "odebug" => self.features.log = true,
            _ if !has_args => match name {
                "prefix" => self.features.prefix = true,
                "bin" => self.features.bin = true,
                "etc" => self.features.etc = true,
                "lib" => self.features.lib = true,
                "libexec" => self.features.libexec = true,
                "pkgetc" => self.features.pkgetc = true,
                "pkgshare" => self.features.pkgshare = true,
                "sbin" => self.features.sbin = true,
                "share" => self.features.share = true,
                "var" => self.features.var = true,
                "name" => self.features.formula_name = true,
                _ => {
                    if self.user_methods.contains(name) {
                        self.features.helper_methods = true;
                    }
                }
            },
            _ => {
                if self.user_methods.contains(name) {
                    self.features.helper_methods = true;
                }
            }
        }
    }

    fn visit_receiver_call(&mut self, name: &str, node: &CallNode<'_>) {
        match name {
            "mkpath" => self.features.mkpath = true,
            "install_symlink" => self.features.install_symlink = true,
            "atomic_write" => self.features.atomic_write = true,
            "chmod" => self.features.chmod = true,
            "install" => self.features.install = true,
            "[]=" if receiver_is_constant(node, "ENV") => self.features.env = true,
            "mac?" | "linux?" => self.features.os_condition = true,
            "exist?" | "symlink?" | "directory?" => self.features.path_condition = true,
            "major_minor" | "major" if receiver_is_version_call(node) => {
                self.features.formula_version = true;
            }
            "safe_popen_read" if receiver_is_constant(node, "Utils") => {
                self.features.process_capture = true;
            }
            "[]" if receiver_is_constant(node, "Formula") => self.features.formula_ref = true,
            _ => {}
        }
    }
}

impl<'pr> Visit<'pr> for PostInstallFeatureCollector<'_> {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        if let Ok(name) = call_name(node) {
            if node.receiver().is_some() {
                self.visit_receiver_call(&name, node);
            } else {
                self.visit_receiverless(&name, node.arguments().is_some());
            }
        }
        ruby_prism::visit_call_node(self, node);
    }

    fn visit_constant_read_node(&mut self, node: &ruby_prism::ConstantReadNode<'pr>) {
        if matches!(
            constant_name(&node.name()).as_deref(),
            Ok("HOMEBREW_PREFIX")
        ) {
            self.features.homebrew_prefix = true;
        }
        ruby_prism::visit_constant_read_node(self, node);
    }
}

// ---------------------------------------------------------------------------
// Combined analysis
// ---------------------------------------------------------------------------

/// Combined analysis result from a single parse of a `post_install` block.
#[derive(Debug, Clone)]
pub struct PostInstallAnalysis {
    /// Feature census of the block.
    pub features: PostInstallFeatures,
    /// Tier 1 lowering result — `Ok(program)` if lowering succeeded.
    pub lowering: Result<Program, AnalysisError>,
}

/// Performs complete `post_install` analysis in a single parse pass.
///
/// Returns `Ok(None)` when no `def post_install` is found (not an error).
/// Returns `Ok(Some(...))` when the method is found; `lowering` may be `Err`
/// even when the feature census succeeds.
///
/// # Errors
///
/// Returns [`AnalysisError::UnsupportedPostInstallSyntax`] only when Prism
/// cannot parse the source at all.
pub fn analyze_post_install_all(
    source: &str,
    formula_version: &str,
) -> Result<Option<PostInstallAnalysis>, AnalysisError> {
    let parsed = parse_source(source)?;
    let methods = build_method_table(&parsed)?;

    let Some(method) = methods.get("post_install") else {
        return Ok(None);
    };
    let Some(body) = method.body.as_ref() else {
        return Ok(None);
    };

    // Build the set of user-defined helper method names.
    let user_methods: BTreeSet<&str> = methods
        .keys()
        .filter(|k| k.as_str() != "post_install")
        .map(String::as_str)
        .collect();

    // Feature census via AST visitor.
    let mut collector = PostInstallFeatureCollector {
        features: PostInstallFeatures::default(),
        user_methods: &user_methods,
    };
    collector.visit(body);
    let features = collector.features;

    // Tier 1 lowering (may fail for unsupported syntax).
    let ctx = LowerCtx {
        parsed: &parsed,
        methods: &methods,
        formula_version,
        tier: LoweringTier::Static,
    };
    let lowering = {
        let mut helper_stack = BTreeSet::new();
        lower_method("post_install", &ctx, &mut helper_stack)
            .map(|stmts| Program { statements: stmts })
    };

    Ok(Some(PostInstallAnalysis { features, lowering }))
}

#[cfg(test)]
mod tests;
