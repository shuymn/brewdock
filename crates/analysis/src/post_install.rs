use std::collections::{BTreeMap, BTreeSet};

use ruby_prism::{ConstantId, Node, ParseResult, parse as parse_ruby};

use crate::error::AnalysisError;

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
    /// Create symlinks for entries matching a glob pattern.
    GlobSymlink {
        /// Source directory to scan.
        source_dir: PathExpr,
        /// Glob pattern.
        pattern: String,
        /// Directory that will contain the symlinks.
        link_dir: PathExpr,
    },
}

/// Path condition for [`Statement::IfPath`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathCondition {
    /// Path exists (file, directory, or symlink).
    Exists,
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
    pub segments: Vec<String>,
}

impl PathExpr {
    /// Creates a new path expression from a base and segment slices.
    #[must_use]
    pub fn new(base: PathBase, segments: &[&str]) -> Self {
        Self {
            base,
            segments: segments.iter().map(|&s| s.to_owned()).collect(),
        }
    }
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
/// # Errors
///
/// Returns [`AnalysisError::UnsupportedPostInstallSyntax`] if the source contains
/// unsupported Ruby constructs.
pub fn lower_post_install(source: &str, formula_version: &str) -> Result<Program, AnalysisError> {
    let parsed = parse_source(source)?;
    let methods = build_method_table(&parsed)?;
    let mut helper_stack = BTreeSet::new();
    let statements = lower_method(
        "post_install",
        &parsed,
        &methods,
        &mut helper_stack,
        formula_version,
    )?;
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

fn lower_method<'pr>(
    name: &str,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
    formula_version: &str,
) -> Result<Vec<Statement>, AnalysisError> {
    let method = methods
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
        if let Some(statements) =
            normalize_method_schema(body, parsed, methods, helper_stack, formula_version)?
        {
            return Ok(statements);
        }
        lower_body_node(body, parsed, methods, helper_stack, formula_version)
    })();

    helper_stack.remove(name);
    result
}

fn normalize_method_schema<'pr>(
    body: &Node<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
    formula_version: &str,
) -> Result<Option<Vec<Statement>>, AnalysisError> {
    if matches_ruby_bundler_cleanup_schema(body, methods, formula_version)? {
        return Ok(Some(normalize_ruby_bundler_cleanup(formula_version)));
    }

    if matches_node_npm_propagation_schema(body, parsed)? {
        return Ok(Some(normalize_node_npm_propagation()));
    }

    if matches_shared_mime_info_schema(body, parsed)? {
        return Ok(Some(normalize_shared_mime_info()));
    }

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
) -> Result<bool, AnalysisError> {
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
) -> Result<Option<(PathExpr, PathExpr)>, AnalysisError> {
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

fn matches_ruby_bundler_cleanup_schema(
    body: &Node<'_>,
    methods: &BTreeMap<String, MethodDef<'_>>,
    formula_version: &str,
) -> Result<bool, AnalysisError> {
    if formula_version.is_empty()
        || !methods.contains_key("api_version")
        || !methods.contains_key("rubygems_bindir")
    {
        return Ok(false);
    }
    let mut has_rm = false;
    let mut has_rm_r = false;
    visit_calls(body, &mut |call| {
        match call_name(call)?.as_str() {
            "rm" => has_rm = true,
            "rm_r" => has_rm_r = true,
            _ => {}
        }
        Ok(())
    })?;
    Ok(has_rm && has_rm_r)
}

fn normalize_ruby_bundler_cleanup(formula_version: &str) -> Vec<Statement> {
    let v = compute_ruby_api_version(formula_version);
    let gems = |sub: &[&str]| {
        let mut segs = vec![
            "lib".to_owned(),
            "ruby".to_owned(),
            "gems".to_owned(),
            v.clone(),
        ];
        segs.extend(sub.iter().map(|&s| s.to_owned()));
        PathExpr {
            base: PathBase::HomebrewPrefix,
            segments: segs,
        }
    };
    vec![
        Statement::RemoveIfExists(gems(&["bin", "bundle"])),
        Statement::RemoveIfExists(gems(&["bin", "bundler"])),
        Statement::GlobRemove {
            dir: gems(&["gems"]),
            pattern: "bundler-*".to_owned(),
        },
    ]
}

fn compute_ruby_api_version(version: &str) -> String {
    let mut parts = version.splitn(3, '.');
    let major = parts.next().unwrap_or(version);
    parts.next().map_or_else(
        || format!("{version}.0"),
        |minor| format!("{major}.{minor}.0"),
    )
}

fn matches_node_npm_propagation_schema(
    body: &Node<'_>,
    parsed: &ParseResult<'_>,
) -> Result<bool, AnalysisError> {
    let source = node_source(parsed, body)?;
    if !source.contains("HOMEBREW_PREFIX") || !source.contains("node_modules") {
        return Ok(false);
    }
    let mut has_cp_r = false;
    let mut has_ln_sf = false;
    visit_calls(body, &mut |call| {
        match call_name(call)?.as_str() {
            "cp_r" => has_cp_r = true,
            "ln_sf" => has_ln_sf = true,
            _ => {}
        }
        Ok(())
    })?;
    Ok(has_cp_r && has_ln_sf)
}

fn normalize_node_npm_propagation() -> Vec<Statement> {
    let hp = |segs: &[&str]| PathExpr::new(PathBase::HomebrewPrefix, segs);
    let bp = |segs: &[&str]| PathExpr::new(PathBase::Bin, segs);
    let node_modules = hp(&["lib", "node_modules"]);
    let npm_dir = hp(&["lib", "node_modules", "npm"]);

    let mut stmts = vec![
        Statement::Mkpath(node_modules.clone()),
        Statement::IfPath {
            condition: npm_dir.clone(),
            kind: PathCondition::Exists,
            then_branch: vec![Statement::RemoveIfExists(npm_dir)],
        },
        Statement::RecursiveCopy {
            from: PathExpr::new(PathBase::Libexec, &["lib", "node_modules", "npm"]),
            to: node_modules,
        },
        Statement::ForceSymlink {
            target: hp(&["lib", "node_modules", "npm", "bin", "npm-cli.js"]),
            link: bp(&["npm"]),
        },
        Statement::ForceSymlink {
            target: hp(&["lib", "node_modules", "npm", "bin", "npx-cli.js"]),
            link: bp(&["npx"]),
        },
        Statement::ForceSymlink {
            target: bp(&["npm"]),
            link: hp(&["bin", "npm"]),
        },
        Statement::ForceSymlink {
            target: bp(&["npx"]),
            link: hp(&["bin", "npx"]),
        },
    ];

    for man_section in &["man1", "man5", "man7"] {
        stmts.push(Statement::Mkpath(hp(&["share", "man", man_section])));
        stmts.push(Statement::GlobRemove {
            dir: hp(&["share", "man", man_section]),
            pattern: "{npm.,npm-,npmrc.,package.json.,npx.}*".to_owned(),
        });
        stmts.push(Statement::GlobSymlink {
            source_dir: hp(&["lib", "node_modules", "npm", "man", man_section]),
            pattern: "{npm,package-,shrinkwrap-,npx}*".to_owned(),
            link_dir: hp(&["share", "man", man_section]),
        });
    }

    stmts.push(Statement::WriteFile {
        path: hp(&["lib", "node_modules", "npm", "npmrc"]),
        content: vec![
            ContentPart::Literal("prefix = ".to_owned()),
            ContentPart::HomebrewPrefix,
            ContentPart::Literal("\n".to_owned()),
        ],
    });

    stmts
}

fn matches_shared_mime_info_schema(
    body: &Node<'_>,
    parsed: &ParseResult<'_>,
) -> Result<bool, AnalysisError> {
    let source = node_source(parsed, body)?;
    if !source.contains("HOMEBREW_PREFIX/\"share/mime\"")
        || !source.contains("ln_sf(global_mime, cellar_mime)")
        || !source.contains("(pkgshare/\"packages\").children")
        || !source.contains("update-mime-database")
    {
        return Ok(false);
    }

    let mut has_rm_r = false;
    let mut has_ln_sf = false;
    let mut has_cp = false;
    let mut has_system = false;
    visit_calls(body, &mut |call| {
        match call_name(call)?.as_str() {
            "rm_r" => has_rm_r = true,
            "ln_sf" => has_ln_sf = true,
            "cp" => has_cp = true,
            "system" => has_system = true,
            _ => {}
        }
        Ok(())
    })?;

    Ok(has_rm_r && has_ln_sf && has_cp && has_system)
}

fn normalize_shared_mime_info() -> Vec<Statement> {
    let hp = |segs: &[&str]| PathExpr::new(PathBase::HomebrewPrefix, segs);
    let share = |segs: &[&str]| PathExpr::new(PathBase::Share, segs);
    let pkgshare = |segs: &[&str]| PathExpr::new(PathBase::Pkgshare, segs);
    let bin = |segs: &[&str]| PathExpr::new(PathBase::Bin, segs);

    let global_mime = hp(&["share", "mime"]);
    let cellar_mime = share(&["mime"]);

    vec![
        Statement::IfPath {
            condition: global_mime.clone(),
            kind: PathCondition::Symlink,
            then_branch: vec![Statement::RemoveIfExists(global_mime.clone())],
        },
        Statement::IfPath {
            condition: cellar_mime.clone(),
            kind: PathCondition::ExistsAndNotSymlink,
            then_branch: vec![Statement::RemoveIfExists(cellar_mime.clone())],
        },
        Statement::ForceSymlink {
            target: global_mime.clone(),
            link: cellar_mime,
        },
        Statement::Mkpath(hp(&["share", "mime", "packages"])),
        Statement::CopyChildren {
            from_dir: pkgshare(&["packages"]),
            to_dir: hp(&["share", "mime", "packages"]),
        },
        Statement::System(vec![
            Argument::Path(bin(&["update-mime-database"])),
            Argument::Path(global_mime),
        ]),
    ]
}

fn lower_body_node<'pr>(
    body: &Node<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
    formula_version: &str,
) -> Result<Vec<Statement>, AnalysisError> {
    let mut statements = Vec::new();
    for child in body_statements(body)? {
        statements.extend(lower_statement(
            &child,
            parsed,
            methods,
            helper_stack,
            formula_version,
        )?);
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
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
    formula_version: &str,
) -> Result<Vec<Statement>, AnalysisError> {
    if let Some(if_node) = node.as_if_node() {
        return lower_if_statement(&if_node, parsed, methods, helper_stack, formula_version);
    }

    if let Some(call) = node.as_call_node() {
        return lower_call_statement(&call, parsed, methods, helper_stack, formula_version);
    }

    if let Some(unless_node) = node.as_unless_node() {
        return lower_unless_statement(
            &unless_node,
            parsed,
            methods,
            helper_stack,
            formula_version,
        );
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
    formula_version: &str,
) -> Result<Vec<Statement>, AnalysisError> {
    if predicate_is_os_runtime(&if_node.predicate(), "mac?")? {
        return lower_statements_node_opt(
            if_node.statements(),
            parsed,
            methods,
            helper_stack,
            formula_version,
        );
    }

    if predicate_is_os_runtime(&if_node.predicate(), "linux?")? {
        return lower_else_branch(
            if_node.subsequent(),
            parsed,
            methods,
            helper_stack,
            formula_version,
        );
    }

    if let Some(condition) =
        parse_exist_condition(&if_node.predicate(), parsed, methods, helper_stack)?
    {
        let then_branch = lower_statements_node_opt(
            if_node.statements(),
            parsed,
            methods,
            helper_stack,
            formula_version,
        )?;
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
        node_source(parsed, &if_node.predicate())?
    ))
}

fn lower_unless_statement<'pr>(
    unless_node: &ruby_prism::UnlessNode<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
    formula_version: &str,
) -> Result<Vec<Statement>, AnalysisError> {
    // `unless OS.mac?` — on macOS OS.mac? is TRUE → unless body never executes → skip
    if predicate_is_os_runtime(&unless_node.predicate(), "mac?")? {
        return Ok(vec![]);
    }
    // `unless OS.linux?` — on macOS OS.linux? is FALSE → unless body always executes
    if predicate_is_os_runtime(&unless_node.predicate(), "linux?")? {
        return lower_statements_node_opt(
            unless_node.statements(),
            parsed,
            methods,
            helper_stack,
            formula_version,
        );
    }
    unsupported(&format!(
        "unsupported unless condition: {}",
        node_source(parsed, &unless_node.predicate())?
    ))
}

fn lower_else_branch<'pr>(
    subsequent: Option<Node<'pr>>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
    formula_version: &str,
) -> Result<Vec<Statement>, AnalysisError> {
    let Some(subsequent) = subsequent else {
        return Ok(Vec::new());
    };
    if let Some(else_node) = subsequent.as_else_node() {
        return lower_statements_node_opt(
            else_node.statements(),
            parsed,
            methods,
            helper_stack,
            formula_version,
        );
    }
    if let Some(if_node) = subsequent.as_if_node() {
        return lower_if_statement(&if_node, parsed, methods, helper_stack, formula_version);
    }
    unsupported("unsupported runtime else branch")
}

fn lower_statements_node_opt<'pr>(
    statements: Option<ruby_prism::StatementsNode<'pr>>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
    formula_version: &str,
) -> Result<Vec<Statement>, AnalysisError> {
    let Some(statements) = statements else {
        return Ok(Vec::new());
    };
    let mut lowered = Vec::new();
    for child in &statements.body() {
        lowered.extend(lower_statement(
            &child,
            parsed,
            methods,
            helper_stack,
            formula_version,
        )?);
    }
    Ok(lowered)
}

fn lower_call_statement<'pr>(
    call: &ruby_prism::CallNode<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
    formula_version: &str,
) -> Result<Vec<Statement>, AnalysisError> {
    let name = call_name(call)?;
    if let Some(receiver) = call.receiver() {
        let receiver = parse_path_expr(&receiver, parsed, methods, helper_stack)?;
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
                            target: parse_path_expr(arg, parsed, methods, helper_stack)?,
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
        "system" | "quiet_system" => {
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
        "mkdir_p" => {
            let arguments = call_args(call);
            if arguments.len() != 1 {
                return unsupported("mkdir_p expects exactly one argument");
            }
            Ok(vec![Statement::Mkpath(parse_path_expr(
                &arguments[0],
                parsed,
                methods,
                helper_stack,
            )?)])
        }
        // Homebrew logging helpers — purely informational, safe to ignore.
        "ohai" | "opoo" | "odebug" => Ok(vec![]),
        helper if methods.contains_key(helper) => {
            lower_method(helper, parsed, methods, helper_stack, formula_version)
        }
        _ => unsupported(&format!("unsupported post_install statement: {name}")),
    }
}

fn parse_argument<'pr>(
    node: &Node<'pr>,
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
    helper_stack: &mut BTreeSet<String>,
) -> Result<Argument, AnalysisError> {
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
) -> Result<PathExpr, AnalysisError> {
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
) -> Result<PathExpr, AnalysisError> {
    if let Some(parentheses) = node.as_parentheses_node()
        && let Some(body) = parentheses.body()
    {
        // ruby-prism wraps parenthesized expressions in StatementsNode;
        // unwrap to the single inner expression when possible.
        if let Some(statements) = body.as_statements_node() {
            let children: Vec<_> = statements.body().iter().collect();
            if children.len() == 1 {
                return parse_path_expr_ast(&children[0], parsed, methods, helper_stack);
            }
        }
        return parse_path_expr_ast(&body, parsed, methods, helper_stack);
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
        path.segments
            .push(validate_path_segment(&segment)?.to_owned());
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
) -> Result<PathExpr, AnalysisError> {
    let method = methods
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
        .map(|segment| validate_path_segment(segment).map(ToOwned::to_owned))
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

fn parse_path_segments(raw: &str) -> Result<Vec<String>, AnalysisError> {
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
        segments.push(validate_path_segment(&stripped[..end])?.to_owned());
        remainder = stripped[end + 1..].trim();
    }
    Ok(segments)
}

fn validate_path_segment(segment: &str) -> Result<&str, AnalysisError> {
    if segment.is_empty() || segment == "." || segment == ".." {
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
    parsed: &ParseResult<'pr>,
    methods: &BTreeMap<String, MethodDef<'pr>>,
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
    Ok(Some(parse_path_expr(
        &receiver,
        parsed,
        methods,
        helper_stack,
    )?))
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
    next.segments.push(segment.to_owned());
    next
}

fn unsupported<T>(message: &str) -> Result<T, AnalysisError> {
    Err(AnalysisError::UnsupportedPostInstallSyntax {
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
    fn test_validate_bundle_bootstrap_with_helper() -> Result<(), Box<dyn std::error::Error>> {
        // Test: pkgetc directly (no helper) — this IS the bundle bootstrap schema
        let direct = r#"
class CaCertificates < Formula
  def post_install
    pkgetc.mkpath
    (pkgetc/"cert.pem").atomic_write(File.read(pkgshare/"cacert.pem"))
  end
end
"#;
        lower_post_install(direct, "2024.01")?;

        // Test: with openssldir helper that resolves to pkgetc
        let with_helper = r#"
class CurlCaBundle < Formula
  def openssldir
    pkgetc
  end
  def post_install
    openssldir.mkpath
    (openssldir/"cert.pem").atomic_write(File.read(pkgshare/"cacert.pem"))
  end
end
"#;
        lower_post_install(with_helper, "2024.01")?;
        Ok(())
    }

    #[test]
    fn test_lower_post_install_rejects_empty_source() {
        let result = lower_post_install("", "1.0");
        assert!(result.is_err());
    }

    #[test]
    fn test_lower_post_install_rejects_unsupported_syntax() {
        let source = r#"
class Demo < Formula
  def post_install
    require "fileutils"
  end
end
"#;
        let result = lower_post_install(source, "1.0");
        assert!(result.is_err());
    }

    #[test]
    fn test_quiet_system_lowered_as_system() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Gnupg < Formula
  def post_install
    (var/"run").mkpath
    quiet_system "killall", "gpg-agent"
  end
end
"#;
        let program = lower_post_install(source, "2.4.1")?;
        assert_eq!(
            program.statements,
            vec![
                Statement::Mkpath(PathExpr::new(PathBase::Var, &["run"])),
                Statement::System(vec![
                    Argument::String("killall".to_owned()),
                    Argument::String("gpg-agent".to_owned()),
                ]),
            ]
        );
        Ok(())
    }

    #[test]
    fn test_atomic_write_string_lowered_as_write_file() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Node22 < Formula
  def post_install
    (lib/"node_modules/npm/npmrc").atomic_write("prefix = #{HOMEBREW_PREFIX}\n")
  end
end
"#;
        let program = lower_post_install(source, "22.0.0")?;
        assert_eq!(
            program.statements,
            vec![Statement::WriteFile {
                path: PathExpr::new(PathBase::Lib, &["node_modules/npm/npmrc"]),
                content: vec![
                    ContentPart::Literal("prefix = ".to_owned()),
                    ContentPart::HomebrewPrefix,
                    ContentPart::Literal("\n".to_owned()),
                ],
            }]
        );
        Ok(())
    }

    #[test]
    fn test_atomic_write_plain_string() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  def post_install
    (etc/"demo.conf").atomic_write("hello\n")
  end
end
"#;
        let program = lower_post_install(source, "1.0")?;
        assert_eq!(
            program.statements,
            vec![Statement::WriteFile {
                path: PathExpr::new(PathBase::Etc, &["demo.conf"]),
                content: vec![ContentPart::Literal("hello\n".to_owned())],
            }]
        );
        Ok(())
    }

    #[test]
    fn test_bundle_bootstrap_atomic_write_still_produces_copy()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class CaCertificates < Formula
  def post_install
    pkgetc.mkpath
    (pkgetc/"cert.pem").atomic_write(File.read(pkgshare/"cacert.pem"))
  end
end
"#;
        let program = lower_post_install(source, "2024.01")?;
        // Schema normalizes this to Mkpath + Copy, not WriteFile
        assert!(
            program
                .statements
                .iter()
                .any(|s| matches!(s, Statement::Copy { .. }))
        );
        assert!(
            !program
                .statements
                .iter()
                .any(|s| matches!(s, Statement::WriteFile { .. }))
        );
        Ok(())
    }

    #[test]
    fn test_return_unless_os_mac_is_skipped() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  def post_install
    return unless OS.mac?
    (var/"run").mkpath
  end
end
"#;
        let program = lower_post_install(source, "1.0")?;
        // The guard `return unless OS.mac?` is skipped on macOS; only mkpath remains.
        assert_eq!(
            program.statements,
            vec![Statement::Mkpath(PathExpr::new(PathBase::Var, &["run"]))]
        );
        Ok(())
    }

    #[test]
    fn test_unless_os_mac_alone_produces_empty_program() -> Result<(), Box<dyn std::error::Error>> {
        // A post_install that only ever runs on non-macOS systems.
        let source = r"
class Demo < Formula
  def post_install
    return unless OS.mac?
  end
end
";
        let program = lower_post_install(source, "1.0")?;
        assert!(program.statements.is_empty());
        Ok(())
    }

    #[test]
    fn test_install_symlink_multiple_args() -> Result<(), Box<dyn std::error::Error>> {
        // (HOMEBREW_PREFIX/"bin").install_symlink bin/"rustup", bin/"rustup-init"
        let source = r#"
class Rustup < Formula
  def post_install
    (HOMEBREW_PREFIX/"bin").install_symlink bin/"rustup", bin/"rustup-init"
  end
end
"#;
        let program = lower_post_install(source, "1.0")?;
        let hp_bin = PathExpr::new(PathBase::HomebrewPrefix, &["bin"]);
        assert_eq!(
            program.statements,
            vec![
                Statement::InstallSymlink {
                    link_dir: hp_bin.clone(),
                    target: PathExpr::new(PathBase::Bin, &["rustup"]),
                },
                Statement::InstallSymlink {
                    link_dir: hp_bin,
                    target: PathExpr::new(PathBase::Bin, &["rustup-init"]),
                },
            ]
        );
        Ok(())
    }

    #[test]
    fn test_mkdir_p_without_receiver() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Glibc < Formula
  def post_install
    mkdir_p lib/"locale"
  end
end
"#;
        let program = lower_post_install(source, "2.39")?;
        assert_eq!(
            program.statements,
            vec![Statement::Mkpath(PathExpr::new(PathBase::Lib, &["locale"]))]
        );
        Ok(())
    }
}
