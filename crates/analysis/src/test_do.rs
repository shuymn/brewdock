use ruby_prism::{BlockNode, CallNode, ConstantId, Node, ParseResult, Visit, parse as parse_ruby};

use crate::error::AnalysisError;

/// Lowered `test do` program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestProgram {
    /// Lowered statements in source order.
    pub statements: Vec<TestStatement>,
}

/// Lowered `test do` statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestStatement {
    /// Assign a simple value to a local variable.
    Assign { variable: String, value: TestExpr },
    /// Write literal/interpolated content into a file under `testpath`.
    WriteFile {
        path: TestPathExpr,
        content: TestExpr,
    },
    /// Run a process and require success.
    System(Vec<TestArg>),
    /// Assert that `actual` contains `expected`.
    AssertMatch {
        expected: TestExpr,
        actual: TestExpr,
    },
    /// Assert string equality.
    AssertEqual {
        expected: TestExpr,
        actual: TestExpr,
    },
    /// Create a directory (and parents) under `testpath`.
    Mkpath(TestPathExpr),
    /// Create an empty file (or update mtime) under `testpath`.
    Touch(TestExpr),
    /// Assert that `actual` does NOT contain `expected`.
    RefuteMatch {
        expected: TestExpr,
        actual: TestExpr,
    },
    /// Assert that a path exists on the filesystem.
    AssertPathExists { path: TestExpr },
    /// Assert that a path does NOT exist on the filesystem.
    RefutePathExists { path: TestExpr },
}

/// Lowered test expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestExpr {
    /// String literal or interpolation.
    String(TestStringExpr),
    /// Filesystem path expression.
    Path(TestPathExpr),
    /// Full formula version string.
    VersionString,
    /// Local variable reference.
    Variable(String),
    /// Command output captured from the shell.
    ShellOutput {
        /// Shell command string.
        command: TestStringExpr,
        /// Optional expected exit code.
        expected_status: Option<i32>,
    },
    /// Command output captured from the shell with stdin piped.
    PipeOutput {
        /// Shell command string.
        command: TestStringExpr,
        /// Content piped to stdin.
        stdin: Box<Self>,
        /// Optional expected exit code.
        expected_status: Option<i32>,
    },
    /// Strip a trailing newline sequence.
    Chomp(Box<Self>),
    /// Strip leading and trailing whitespace.
    Strip(Box<Self>),
    /// Read the contents of a file.
    ReadFile(TestPathExpr),
}

/// String expression with literal and interpolated parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestStringExpr {
    /// Ordered parts.
    pub parts: Vec<TestStringPart>,
}

impl TestStringExpr {
    /// Creates a single literal string expression.
    #[must_use]
    pub fn literal(value: &str) -> Self {
        Self {
            parts: vec![TestStringPart::Literal(value.to_owned())],
        }
    }
}

/// A string expression part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestStringPart {
    /// Literal text.
    Literal(String),
    /// Interpolated path rendered as a string.
    Path(TestPathExpr),
    /// The formula version string.
    VersionString,
}

/// A command argument for `system`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestArg {
    /// String argument.
    String(TestStringExpr),
    /// Path argument.
    Path(TestPathExpr),
    /// Variable reference.
    Variable(String),
}

/// Allowed path bases in the v1 `test do` runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestPathBase {
    /// Temporary test directory.
    Testpath,
    /// Installed keg root.
    Prefix,
    /// `prefix/bin`.
    Bin,
    /// `prefix/include`.
    Include,
    /// `prefix/lib`.
    Lib,
    /// `prefix/libexec`.
    Libexec,
    /// `prefix/share/<formula_name>`.
    Pkgshare,
    /// `prefix/sbin`.
    Sbin,
    /// `prefix/share`.
    Share,
}

/// Allowed path expression in the v1 `test do` runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestPathExpr {
    /// Base path.
    pub base: TestPathBase,
    /// Literal path segments.
    pub segments: Vec<String>,
}

/// Feature census for a formula `test do` block.
#[allow(clippy::struct_excessive_bools)] // each bool maps to a distinct language construct
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestDoFeatures {
    /// Uses `assert_match`.
    pub assert_match: bool,
    /// Uses `assert_equal`.
    pub assert_equal: bool,
    /// Uses `assert_path_exists`.
    pub assert_path_exists: bool,
    /// Uses `assert_predicate`.
    pub assert_predicate: bool,
    /// Uses `refute_match`.
    pub refute_match: bool,
    /// Uses `shell_output`.
    pub shell_output: bool,
    /// Uses `pipe_output`.
    pub pipe_output: bool,
    /// Uses `system`.
    pub system: bool,
    /// Uses `testpath`.
    pub testpath: bool,
    /// Uses `bin`.
    pub bin: bool,
    /// Uses `prefix`.
    pub prefix: bool,
    /// Uses `pkgshare`.
    pub pkgshare: bool,
    /// Uses `version`.
    pub version: bool,
    /// Uses `ENV`.
    pub env: bool,
    /// Uses `resource`.
    pub resource: bool,
    /// Uses `require`.
    pub require: bool,
    /// Uses `free_port`.
    pub free_port: bool,
    /// Uses `cp_r`.
    pub cp_r: bool,
    /// Uses `.write`.
    pub write: bool,
    /// Uses `.mkpath`.
    pub mkpath: bool,
    /// Uses `.touch`.
    pub touch: bool,
    /// Uses `.chomp`.
    pub chomp: bool,
}

impl TestDoFeatures {
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
        push_if!(assert_match, "assert_match");
        push_if!(assert_equal, "assert_equal");
        push_if!(assert_path_exists, "assert_path_exists");
        push_if!(assert_predicate, "assert_predicate");
        push_if!(refute_match, "refute_match");
        push_if!(shell_output, "shell_output");
        push_if!(pipe_output, "pipe_output");
        push_if!(system, "system");
        push_if!(testpath, "testpath");
        push_if!(bin, "bin");
        push_if!(prefix, "prefix");
        push_if!(pkgshare, "pkgshare");
        push_if!(version, "version");
        push_if!(env, "ENV");
        push_if!(resource, "resource");
        push_if!(require, "require");
        push_if!(free_port, "free_port");
        push_if!(cp_r, "cp_r");
        push_if!(write, "write");
        push_if!(mkpath, "mkpath");
        push_if!(touch, "touch");
        push_if!(chomp, "chomp");
        names
    }
}

/// Extracts the contents of `test do ... end`.
///
/// # Errors
///
/// Returns [`AnalysisError::UnsupportedTestDoSyntax`] when the block is
/// missing or cannot be matched in the parsed source.
pub fn extract_test_do_block(source: &str) -> Result<String, AnalysisError> {
    let parsed = parse_source(source)?;
    let block =
        find_test_block(&parsed)?.ok_or_else(|| unsupported_test("missing test do block"))?;
    let body = block
        .body()
        .ok_or_else(|| unsupported_test("missing test do body"))?;
    node_source(&parsed, &body).map(ToOwned::to_owned)
}

/// Validates that a `test do` block can be lowered into the v1 runtime IR.
///
/// # Errors
///
/// Returns [`AnalysisError::UnsupportedTestDoSyntax`] when Prism cannot parse
/// the source, the `test do` block is missing, or the runtime subset is not supported.
pub fn validate_test_do(source: &str) -> Result<(), AnalysisError> {
    let _program = lower_test_do(source)?;
    Ok(())
}

/// Parses a `test do` block and returns a feature census.
///
/// # Errors
///
/// Returns [`AnalysisError::UnsupportedTestDoSyntax`] when Prism cannot parse
/// the source or the `test do` block is missing.
pub fn analyze_test_do(source: &str) -> Result<TestDoFeatures, AnalysisError> {
    let parsed = parse_source(source)?;
    let block =
        find_test_block(&parsed)?.ok_or_else(|| unsupported_test("missing test do block"))?;
    let mut collector = TestDoFeatureCollector::default();
    collector.visit(&block.as_node());
    Ok(collector.features)
}

/// Combined analysis result from a single parse of a `test do` block.
#[derive(Debug, Clone)]
pub struct TestDoAnalysis {
    /// Feature census of the block.
    pub features: TestDoFeatures,
    /// v1 runtime lowering result — `Ok(program)` if lowering succeeded.
    pub lowering: Result<TestProgram, AnalysisError>,
}

/// Performs complete `test do` analysis in a single parse pass.
///
/// Returns `Ok(None)` when no `test do` block is found (not an error).
/// Returns `Ok(Some(...))` when a block is found; `lowering` may be `Err`
/// even when the feature census succeeds.
///
/// # Errors
///
/// Returns [`AnalysisError::UnsupportedTestDoSyntax`] only when Prism cannot
/// parse the source at all.
pub fn analyze_test_do_all(source: &str) -> Result<Option<TestDoAnalysis>, AnalysisError> {
    let parsed = parse_source(source)?;
    let Some(block) = find_test_block(&parsed)? else {
        return Ok(None);
    };

    let mut collector = TestDoFeatureCollector::default();
    collector.visit(&block.as_node());
    let features = collector.features;

    let lowering = block.body().map_or_else(
        || Err(unsupported_test("missing test do body")),
        |body| lower_body_node(&parsed, &body).map(|stmts| TestProgram { statements: stmts }),
    );

    Ok(Some(TestDoAnalysis { features, lowering }))
}

/// Lowers a `test do` block into the v1 runtime IR.
///
/// # Errors
///
/// Returns [`AnalysisError::UnsupportedTestDoSyntax`] when the test uses syntax
/// outside the supported subset.
pub fn lower_test_do(source: &str) -> Result<TestProgram, AnalysisError> {
    let parsed = parse_source(source)?;
    let block =
        find_test_block(&parsed)?.ok_or_else(|| unsupported_test("missing test do block"))?;
    let Some(body) = block.body() else {
        return Err(unsupported_test("missing test do body"));
    };
    let statements = lower_body_node(&parsed, &body)?;
    Ok(TestProgram { statements })
}

fn parse_source(source: &str) -> Result<ParseResult<'_>, AnalysisError> {
    let parsed = parse_ruby(source.as_bytes());
    if let Some(error) = parsed.errors().next() {
        return Err(unsupported_test(&format!(
            "prism parse error: {}",
            error.message()
        )));
    }
    Ok(parsed)
}

fn lower_body_node(
    parsed: &ParseResult<'_>,
    body: &Node<'_>,
) -> Result<Vec<TestStatement>, AnalysisError> {
    let Some(statements) = body.as_statements_node() else {
        return Err(unsupported_test("unsupported test do body"));
    };
    lower_statements_opt(parsed, Some(statements))
}

fn predicate_is_os_call(node: &Node<'_>, method: &str) -> Result<bool, AnalysisError> {
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
    Ok(constant_name(&receiver.name())? == "OS")
}

fn lower_statements_opt(
    parsed: &ParseResult<'_>,
    statements: Option<ruby_prism::StatementsNode<'_>>,
) -> Result<Vec<TestStatement>, AnalysisError> {
    let Some(statements) = statements else {
        return Ok(Vec::new());
    };
    let mut lowered = Vec::new();
    for child in &statements.body() {
        if let Some(if_node) = child.as_if_node() {
            lowered.extend(lower_if_os(parsed, &if_node)?);
            continue;
        }
        if let Some(unless_node) = child.as_unless_node() {
            lowered.extend(lower_unless_os(parsed, &unless_node)?);
            continue;
        }
        lowered.push(lower_statement(parsed, &child)?);
    }
    Ok(lowered)
}

fn lower_if_os(
    parsed: &ParseResult<'_>,
    if_node: &ruby_prism::IfNode<'_>,
) -> Result<Vec<TestStatement>, AnalysisError> {
    // `if OS.mac?` — on macOS: true → take then-branch
    if predicate_is_os_call(&if_node.predicate(), "mac?")? {
        return lower_statements_opt(parsed, if_node.statements());
    }
    // `if OS.linux?` — on macOS: false → take else-branch
    if predicate_is_os_call(&if_node.predicate(), "linux?")? {
        return lower_else_branch(parsed, if_node.subsequent());
    }
    Err(unsupported_test(&format!(
        "unsupported test do statement: {}",
        node_source(parsed, &if_node.predicate())?
    )))
}

fn lower_unless_os(
    parsed: &ParseResult<'_>,
    unless_node: &ruby_prism::UnlessNode<'_>,
) -> Result<Vec<TestStatement>, AnalysisError> {
    // `unless OS.mac?` — on macOS: true → skip body
    if predicate_is_os_call(&unless_node.predicate(), "mac?")? {
        return Ok(Vec::new());
    }
    // `unless OS.linux?` — on macOS: false → take body
    if predicate_is_os_call(&unless_node.predicate(), "linux?")? {
        return lower_statements_opt(parsed, unless_node.statements());
    }
    Err(unsupported_test(&format!(
        "unsupported test do statement: {}",
        node_source(parsed, &unless_node.predicate())?
    )))
}

fn lower_else_branch(
    parsed: &ParseResult<'_>,
    subsequent: Option<Node<'_>>,
) -> Result<Vec<TestStatement>, AnalysisError> {
    let Some(subsequent) = subsequent else {
        return Ok(Vec::new());
    };
    if let Some(else_node) = subsequent.as_else_node() {
        return lower_statements_opt(parsed, else_node.statements());
    }
    // elsif → treat as nested if
    if let Some(if_node) = subsequent.as_if_node() {
        return lower_if_os(parsed, &if_node);
    }
    Err(unsupported_test("unsupported else branch"))
}

fn lower_statement(
    parsed: &ParseResult<'_>,
    node: &Node<'_>,
) -> Result<TestStatement, AnalysisError> {
    if let Some(assign) = node.as_local_variable_write_node() {
        return Ok(TestStatement::Assign {
            variable: constant_name(&assign.name())?,
            value: parse_expr(parsed, &assign.value())?,
        });
    }

    if let Some(call) = node.as_call_node() {
        return lower_call_statement(parsed, &call);
    }

    Err(unsupported_test(&format!(
        "unsupported test do statement: {}",
        node_source(parsed, node)?
    )))
}

fn lower_call_statement(
    parsed: &ParseResult<'_>,
    call: &CallNode<'_>,
) -> Result<TestStatement, AnalysisError> {
    let name = call_name(call)?;
    if let Some(receiver) = call.receiver() {
        let receiver = parse_expr(parsed, &receiver)?;
        return match name.as_str() {
            "write" => {
                let arguments = call_args(call);
                if arguments.len() != 1 {
                    return Err(unsupported_test("write expects exactly one argument"));
                }
                let TestExpr::Path(path) = receiver else {
                    return Err(unsupported_test("write receiver must be a path"));
                };
                Ok(TestStatement::WriteFile {
                    path,
                    content: parse_expr(parsed, &arguments[0])?,
                })
            }
            "mkpath" => {
                let TestExpr::Path(path) = receiver else {
                    return Err(unsupported_test("mkpath receiver must be a path"));
                };
                Ok(TestStatement::Mkpath(path))
            }
            _ => Err(unsupported_test(&format!(
                "unsupported test do call: {name}"
            ))),
        };
    }

    match name.as_str() {
        "system" => {
            let arguments = call_args(call);
            if arguments.is_empty() {
                return Err(unsupported_test("system expects at least one argument"));
            }
            Ok(TestStatement::System(
                arguments
                    .iter()
                    .map(|arg| parse_arg(parsed, arg))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        "assert_match" => {
            let arguments = call_args(call);
            if arguments.len() != 2 {
                return Err(unsupported_test(
                    "assert_match expects exactly two arguments",
                ));
            }
            Ok(TestStatement::AssertMatch {
                expected: parse_expr(parsed, &arguments[0])?,
                actual: parse_expr(parsed, &arguments[1])?,
            })
        }
        "assert_equal" => {
            let arguments = call_args(call);
            if arguments.len() != 2 {
                return Err(unsupported_test(
                    "assert_equal expects exactly two arguments",
                ));
            }
            Ok(TestStatement::AssertEqual {
                expected: parse_expr(parsed, &arguments[0])?,
                actual: parse_expr(parsed, &arguments[1])?,
            })
        }
        "refute_match" => {
            let arguments = call_args(call);
            if arguments.len() != 2 {
                return Err(unsupported_test(
                    "refute_match expects exactly two arguments",
                ));
            }
            Ok(TestStatement::RefuteMatch {
                expected: parse_expr(parsed, &arguments[0])?,
                actual: parse_expr(parsed, &arguments[1])?,
            })
        }
        "assert_path_exists" => {
            let arguments = call_args(call);
            if arguments.is_empty() || arguments.len() > 2 {
                return Err(unsupported_test("assert_path_exists expects 1-2 arguments"));
            }
            Ok(TestStatement::AssertPathExists {
                path: parse_expr(parsed, &arguments[0])?,
            })
        }
        "refute_path_exists" => {
            let arguments = call_args(call);
            if arguments.is_empty() || arguments.len() > 2 {
                return Err(unsupported_test("refute_path_exists expects 1-2 arguments"));
            }
            Ok(TestStatement::RefutePathExists {
                path: parse_expr(parsed, &arguments[0])?,
            })
        }
        "touch" => {
            let arguments = call_args(call);
            if arguments.len() != 1 {
                return Err(unsupported_test("touch expects exactly one argument"));
            }
            Ok(TestStatement::Touch(parse_expr(parsed, &arguments[0])?))
        }
        _ => Err(unsupported_test(&format!(
            "unsupported test do statement: {name}"
        ))),
    }
}

fn parse_arg(parsed: &ParseResult<'_>, node: &Node<'_>) -> Result<TestArg, AnalysisError> {
    if let Ok(path) = parse_path_expr(parsed, node) {
        return Ok(TestArg::Path(path));
    }
    if let Some(read) = node.as_local_variable_read_node() {
        return Ok(TestArg::Variable(constant_name(&read.name())?));
    }
    Ok(TestArg::String(parse_string_expr(parsed, node)?))
}

fn parse_expr(parsed: &ParseResult<'_>, node: &Node<'_>) -> Result<TestExpr, AnalysisError> {
    if let Ok(path) = parse_path_expr(parsed, node) {
        return Ok(TestExpr::Path(path));
    }
    if let Some(read) = node.as_local_variable_read_node() {
        return Ok(TestExpr::Variable(constant_name(&read.name())?));
    }
    if let Some(string) = node.as_string_node() {
        return Ok(TestExpr::String(TestStringExpr {
            parts: vec![TestStringPart::Literal(
                String::from_utf8(string.unescaped().to_vec()).map_err(|error| {
                    unsupported_test(&format!("invalid utf-8 string literal: {error}"))
                })?,
            )],
        }));
    }
    if node.as_interpolated_string_node().is_some() {
        return Ok(TestExpr::String(parse_string_expr(parsed, node)?));
    }
    if let Some(call) = node.as_call_node() {
        let name = call_name(&call)?;
        if name == "shell_output" {
            let arguments = call_args(&call);
            if !(1..=2).contains(&arguments.len()) {
                return Err(unsupported_test(
                    "shell_output expects one command and optional status",
                ));
            }
            let expected_status = if arguments.len() == 2 {
                Some(parse_integer(&arguments[1])?)
            } else {
                None
            };
            return Ok(TestExpr::ShellOutput {
                command: parse_string_expr(parsed, &arguments[0])?,
                expected_status,
            });
        }
        if name == "pipe_output" {
            let arguments = call_args(&call);
            if !(1..=3).contains(&arguments.len()) {
                return Err(unsupported_test("pipe_output expects 1-3 arguments"));
            }
            let command = parse_string_expr(parsed, &arguments[0])?;
            let stdin = Box::new(if arguments.len() >= 2 {
                parse_expr(parsed, &arguments[1])?
            } else {
                TestExpr::String(TestStringExpr::literal(""))
            });
            let expected_status = if arguments.len() == 3 {
                Some(parse_integer(&arguments[2])?)
            } else {
                None
            };
            return Ok(TestExpr::PipeOutput {
                command,
                stdin,
                expected_status,
            });
        }
        if name == "version" && call.receiver().is_none() && call.arguments().is_none() {
            return Ok(TestExpr::VersionString);
        }
        if name == "to_s"
            && let Some(receiver) = call.receiver()
            && let Some(version_call) = receiver.as_call_node()
            && call_name(&version_call)? == "version"
            && version_call.receiver().is_none()
            && version_call.arguments().is_none()
        {
            return Ok(TestExpr::VersionString);
        }
        if name == "chomp" {
            let Some(receiver) = call.receiver() else {
                return Err(unsupported_test("chomp requires receiver"));
            };
            return Ok(TestExpr::Chomp(Box::new(parse_expr(parsed, &receiver)?)));
        }
        if name == "strip" {
            let Some(receiver) = call.receiver() else {
                return Err(unsupported_test("strip requires receiver"));
            };
            return Ok(TestExpr::Strip(Box::new(parse_expr(parsed, &receiver)?)));
        }
        if name == "read"
            && call.arguments().is_none()
            && let Some(receiver) = call.receiver()
            && let Ok(path) = parse_path_expr(parsed, &receiver)
        {
            return Ok(TestExpr::ReadFile(path));
        }
    }
    if let Some(regex) = node.as_regular_expression_node() {
        let pattern = String::from_utf8(regex.unescaped().to_vec())
            .map_err(|error| unsupported_test(&format!("invalid utf-8 regex literal: {error}")))?;
        return Ok(TestExpr::String(TestStringExpr::literal(&pattern)));
    }
    Err(unsupported_test(&format!(
        "unsupported test do expression: {}",
        node_source(parsed, node)?
    )))
}

fn parse_string_expr(
    parsed: &ParseResult<'_>,
    node: &Node<'_>,
) -> Result<TestStringExpr, AnalysisError> {
    if let Some(string) = node.as_string_node() {
        let value = String::from_utf8(string.unescaped().to_vec())
            .map_err(|error| unsupported_test(&format!("invalid utf-8 string literal: {error}")))?;
        return Ok(TestStringExpr::literal(&value));
    }

    let Some(interp) = node.as_interpolated_string_node() else {
        return Err(unsupported_test(&format!(
            "expected string expression, got {}",
            node_source(parsed, node)?
        )));
    };

    let mut parts = Vec::new();
    for part in &interp.parts() {
        if let Some(string) = part.as_string_node() {
            let value = String::from_utf8(string.unescaped().to_vec()).map_err(|error| {
                unsupported_test(&format!("invalid utf-8 interpolated string: {error}"))
            })?;
            if !value.is_empty() {
                parts.push(TestStringPart::Literal(value));
            }
            continue;
        }
        let Some(embedded) = part.as_embedded_statements_node() else {
            return Err(unsupported_test("unsupported interpolated string part"));
        };
        let Some(statements) = embedded.statements() else {
            return Err(unsupported_test("empty interpolation"));
        };
        let body: Vec<_> = statements.body().iter().collect();
        if body.len() != 1 {
            return Err(unsupported_test(
                "interpolation must contain one expression",
            ));
        }
        if let Ok(path) = parse_path_expr(parsed, &body[0]) {
            parts.push(TestStringPart::Path(path));
            continue;
        }
        match parse_expr(parsed, &body[0])? {
            TestExpr::VersionString => parts.push(TestStringPart::VersionString),
            _ => return Err(unsupported_test("unsupported interpolated expression")),
        }
    }
    Ok(TestStringExpr { parts })
}

fn parse_path_expr(
    parsed: &ParseResult<'_>,
    node: &Node<'_>,
) -> Result<TestPathExpr, AnalysisError> {
    if let Some(parentheses) = node.as_parentheses_node()
        && let Some(body) = parentheses.body()
    {
        if let Some(statements) = body.as_statements_node() {
            let children: Vec<_> = statements.body().iter().collect();
            if children.len() == 1 {
                return parse_path_expr(parsed, &children[0]);
            }
        }
        return parse_path_expr(parsed, &body);
    }

    if let Some(call) = node.as_call_node() {
        let name = call_name(&call)?;
        if name == "/" {
            let Some(receiver) = call.receiver() else {
                return Err(unsupported_test("path join requires receiver"));
            };
            let mut path = parse_path_expr(parsed, &receiver)?;
            let arguments = call_args(&call);
            if arguments.len() != 1 {
                return Err(unsupported_test("path join expects exactly one segment"));
            }
            let Some(segment) = parse_string_literal(&arguments[0])? else {
                return Err(unsupported_test(
                    "path join segment must be a string literal",
                ));
            };
            path.segments.extend(parse_segments(&segment)?);
            return Ok(path);
        }
    }

    let Some(call) = node.as_call_node() else {
        return Err(unsupported_test(&format!(
            "unsupported path expression: {}",
            node_source(parsed, node)?
        )));
    };

    if call.receiver().is_some() || call.arguments().is_some() {
        return Err(unsupported_test(&format!(
            "unsupported path expression: {}",
            node_source(parsed, node)?
        )));
    }

    let base = match call_name(&call)?.as_str() {
        "testpath" => TestPathBase::Testpath,
        "prefix" => TestPathBase::Prefix,
        "bin" => TestPathBase::Bin,
        "include" => TestPathBase::Include,
        "lib" => TestPathBase::Lib,
        "libexec" => TestPathBase::Libexec,
        "pkgshare" => TestPathBase::Pkgshare,
        "sbin" => TestPathBase::Sbin,
        "share" => TestPathBase::Share,
        _ => {
            return Err(unsupported_test(&format!(
                "unsupported path base: {}",
                node_source(parsed, node)?
            )));
        }
    };
    Ok(TestPathExpr {
        base,
        segments: Vec::new(),
    })
}

fn parse_segments(segment: &str) -> Result<Vec<String>, AnalysisError> {
    segment
        .split('/')
        .filter(|part| !part.is_empty())
        .map(|part| {
            if part == "." || part == ".." {
                return Err(unsupported_test(&format!(
                    "unsupported path segment: {part}"
                )));
            }
            Ok(part.to_owned())
        })
        .collect()
}

fn parse_string_literal(node: &Node<'_>) -> Result<Option<String>, AnalysisError> {
    let Some(string) = node.as_string_node() else {
        return Ok(None);
    };
    String::from_utf8(string.unescaped().to_vec())
        .map(Some)
        .map_err(|error| unsupported_test(&format!("invalid utf-8 string literal: {error}")))
}

fn parse_integer(node: &Node<'_>) -> Result<i32, AnalysisError> {
    let integer = node
        .as_integer_node()
        .ok_or_else(|| unsupported_test("expected integer literal"))?;
    integer
        .value()
        .try_into()
        .map_err(|()| unsupported_test("integer literal out of range"))
}

fn find_test_block<'pr>(
    parsed: &'pr ParseResult<'pr>,
) -> Result<Option<BlockNode<'pr>>, AnalysisError> {
    let mut finder = TestDoFinder::default();
    finder.visit(&parsed.node());
    if let Some(error) = finder.error {
        return Err(error);
    }
    Ok(finder.block)
}

#[derive(Default)]
struct TestDoFinder<'pr> {
    block: Option<BlockNode<'pr>>,
    error: Option<AnalysisError>,
}

impl<'pr> Visit<'pr> for TestDoFinder<'pr> {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        if self.block.is_none() && self.error.is_none() {
            match call_name(node) {
                Ok(name) if name == "test" && node.receiver().is_none() => {
                    self.block = node.block().and_then(|block| block.as_block_node());
                    if self.block.is_some() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    self.error = Some(error);
                    return;
                }
            }
        }
        ruby_prism::visit_call_node(self, node);
    }
}

#[derive(Default)]
struct TestDoFeatureCollector {
    features: TestDoFeatures,
}

impl<'pr> Visit<'pr> for TestDoFeatureCollector {
    fn visit_call_node(&mut self, node: &CallNode<'pr>) {
        if let Ok(name) = call_name(node) {
            match name.as_str() {
                "assert_match" => self.features.assert_match = true,
                "assert_equal" => self.features.assert_equal = true,
                "assert_path_exists" => self.features.assert_path_exists = true,
                "assert_predicate" => self.features.assert_predicate = true,
                "refute_match" => self.features.refute_match = true,
                "shell_output" => self.features.shell_output = true,
                "pipe_output" => self.features.pipe_output = true,
                "system" => self.features.system = true,
                "testpath" => self.features.testpath = true,
                "bin" => self.features.bin = true,
                "prefix" => self.features.prefix = true,
                "pkgshare" => self.features.pkgshare = true,
                "version" => self.features.version = true,
                "resource" => self.features.resource = true,
                "require" => self.features.require = true,
                "free_port" => self.features.free_port = true,
                "cp_r" => self.features.cp_r = true,
                "write" => self.features.write = true,
                "mkpath" => self.features.mkpath = true,
                "touch" => self.features.touch = true,
                "chomp" => self.features.chomp = true,
                "[]=" => {
                    if let Some(receiver) = node.receiver()
                        && let Some(constant) = receiver.as_constant_read_node()
                        && matches!(constant_name(&constant.name()).as_deref(), Ok("ENV"))
                    {
                        self.features.env = true;
                    }
                }
                _ => {}
            }
        }
        ruby_prism::visit_call_node(self, node);
    }
}

fn call_args<'pr>(call: &CallNode<'pr>) -> Vec<Node<'pr>> {
    call.arguments()
        .map(|arguments| arguments.arguments().iter().collect())
        .unwrap_or_default()
}

fn call_name(call: &CallNode<'_>) -> Result<String, AnalysisError> {
    constant_name(&call.name())
}

fn constant_name(id: &ConstantId<'_>) -> Result<String, AnalysisError> {
    std::str::from_utf8(id.as_slice())
        .map(ToOwned::to_owned)
        .map_err(|error| unsupported_test(&format!("invalid prism identifier utf-8: {error}")))
}

fn node_source<'pr>(
    parsed: &ParseResult<'pr>,
    node: &Node<'pr>,
) -> Result<&'pr str, AnalysisError> {
    std::str::from_utf8(parsed.as_slice(&node.location()))
        .map_err(|error| unsupported_test(&format!("invalid source utf-8: {error}")))
}

fn unsupported_test(message: &str) -> AnalysisError {
    AnalysisError::UnsupportedTestDoSyntax {
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_test_do_block() -> Result<(), Box<dyn std::error::Error>> {
        let source = r##"
class Shfmt < Formula
  test do
    assert_match version.to_s, shell_output("#{bin}/shfmt --version")

    (testpath/"test").write "\t\techo foo"
    system bin/"shfmt", testpath/"test"
  end
end
"##;

        let block = extract_test_do_block(source)?;

        assert!(block.contains("assert_match version.to_s"));
        assert!(block.contains("system bin/\"shfmt\""));
        Ok(())
    }

    #[test]
    fn test_analyze_test_do_features() -> Result<(), Box<dyn std::error::Error>> {
        let source = r##"
class Hello < Formula
  test do
    output = shell_output("#{bin}/hello --greeting=brew").chomp
    assert_equal "brew", output
  end
end
"##;

        let features = analyze_test_do(source)?;

        assert!(features.assert_equal);
        assert!(features.shell_output);
        assert!(features.bin);
        assert!(features.chomp);
        Ok(())
    }

    #[test]
    fn test_lower_test_do_shfmt_subset() -> Result<(), Box<dyn std::error::Error>> {
        let source = r##"
class Shfmt < Formula
  test do
    assert_match version.to_s, shell_output("#{bin}/shfmt --version")
    (testpath/"test").write "\t\techo foo"
    system bin/"shfmt", testpath/"test"
  end
end
"##;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 3);
        assert!(matches!(
            program.statements[0],
            TestStatement::AssertMatch {
                expected: TestExpr::VersionString,
                actual: TestExpr::ShellOutput { .. }
            }
        ));
        assert!(matches!(
            program.statements[1],
            TestStatement::WriteFile { .. }
        ));
        assert!(matches!(program.statements[2], TestStatement::System(_)));
        Ok(())
    }

    #[test]
    fn test_lower_test_do_variable_and_chomp() -> Result<(), Box<dyn std::error::Error>> {
        let source = r##"
class Hello < Formula
  test do
    output = shell_output("#{bin}/hello --greeting=brew").chomp
    assert_equal "brew", output
  end
end
"##;

        let program = lower_test_do(source)?;

        assert!(matches!(
            &program.statements[0],
            TestStatement::Assign {
                variable,
                value: TestExpr::Chomp(_)
            } if variable == "output"
        ));
        assert!(matches!(
            &program.statements[1],
            TestStatement::AssertEqual {
                expected: TestExpr::String(_),
                actual: TestExpr::Variable(name)
            } if name == "output"
        ));
        Ok(())
    }

    #[test]
    fn test_lower_test_do_rejects_env_assignment() {
        let source = r#"
class Demo < Formula
  test do
    ENV["FOO"] = "bar"
  end
end
"#;

        let result = lower_test_do(source);

        assert!(matches!(
            result,
            Err(AnalysisError::UnsupportedTestDoSyntax { .. })
        ));
    }

    #[test]
    fn test_analyze_all_no_block() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  def install
    system "make"
  end
end
"#;

        let result = analyze_test_do_all(source)?;

        assert!(result.is_none());
        Ok(())
    }

    #[test]
    fn test_analyze_all_lowerable() -> Result<(), Box<dyn std::error::Error>> {
        let source = r##"
class Shfmt < Formula
  test do
    assert_match version.to_s, shell_output("#{bin}/shfmt --version")
    (testpath/"test").write "\t\techo foo"
    system bin/"shfmt", testpath/"test"
  end
end
"##;

        let analysis = analyze_test_do_all(source)?.ok_or("should find block")?;

        assert!(analysis.features.assert_match);
        assert!(analysis.features.shell_output);
        assert!(analysis.features.bin);
        assert!(analysis.features.system);
        assert!(analysis.features.testpath);
        assert!(analysis.features.write);
        assert!(analysis.lowering.is_ok());
        assert_eq!(
            analysis.lowering.as_ref().ok().map(|p| p.statements.len()),
            Some(3)
        );
        Ok(())
    }

    #[test]
    fn test_analyze_all_unlowerable() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  test do
    ENV["FOO"] = "bar"
  end
end
"#;

        let analysis = analyze_test_do_all(source)?.ok_or("should find block")?;

        assert!(analysis.features.env);
        assert!(analysis.lowering.is_err());
        Ok(())
    }

    #[test]
    fn test_analyze_all_parse_error() {
        let source = "{{{{not valid ruby at all";

        let result = analyze_test_do_all(source);

        assert!(matches!(
            result,
            Err(AnalysisError::UnsupportedTestDoSyntax { .. })
        ));
    }

    #[test]
    fn test_lower_bare_version_in_assert_match() -> Result<(), Box<dyn std::error::Error>> {
        let source = r##"
class Gh < Formula
  test do
    assert_match version, shell_output("#{bin}/gh --version")
  end
end
"##;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 1);
        assert!(matches!(
            &program.statements[0],
            TestStatement::AssertMatch {
                expected: TestExpr::VersionString,
                ..
            }
        ));
        Ok(())
    }

    #[test]
    fn test_lower_bare_version_in_interpolation() -> Result<(), Box<dyn std::error::Error>> {
        let source = r##"
class Demo < Formula
  test do
    assert_match "v#{version}", shell_output("#{bin}/demo --version")
  end
end
"##;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 1);
        if let TestStatement::AssertMatch { expected, .. } = &program.statements[0] {
            assert!(
                matches!(expected, TestExpr::String(s) if s.parts.iter().any(|p| matches!(p, TestStringPart::VersionString)))
            );
        } else {
            return Err("expected AssertMatch".into());
        }
        Ok(())
    }

    #[test]
    fn test_lower_refute_match() -> Result<(), Box<dyn std::error::Error>> {
        let source = r##"
class Demo < Formula
  test do
    refute_match "error", shell_output("#{bin}/demo --check")
  end
end
"##;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 1);
        assert!(matches!(
            &program.statements[0],
            TestStatement::RefuteMatch { .. }
        ));
        Ok(())
    }

    #[test]
    fn test_lower_assert_path_exists() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  test do
    system bin/"demo", "--init"
    assert_path_exists testpath/"output.txt"
  end
end
"#;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 2);
        assert!(matches!(
            &program.statements[1],
            TestStatement::AssertPathExists {
                path: TestExpr::Path(_),
            }
        ));
        Ok(())
    }

    #[test]
    fn test_lower_assert_path_exists_with_message() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  test do
    assert_path_exists testpath/"output.txt", "output should exist"
  end
end
"#;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 1);
        assert!(matches!(
            &program.statements[0],
            TestStatement::AssertPathExists { .. }
        ));
        Ok(())
    }

    #[test]
    fn test_lower_refute_path_exists() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  test do
    refute_path_exists testpath/"should_not_exist"
  end
end
"#;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 1);
        assert!(matches!(
            &program.statements[0],
            TestStatement::RefutePathExists {
                path: TestExpr::Path(_),
            }
        ));
        Ok(())
    }

    #[test]
    fn test_lower_mkpath() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  test do
    (testpath/"sub/dir").mkpath
  end
end
"#;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 1);
        assert!(matches!(&program.statements[0], TestStatement::Mkpath(_)));
        Ok(())
    }

    #[test]
    fn test_lower_touch() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  test do
    touch testpath/"foo_file"
  end
end
"#;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 1);
        assert!(matches!(&program.statements[0], TestStatement::Touch(_)));
        Ok(())
    }

    #[test]
    fn test_lower_pipe_output_two_args() -> Result<(), Box<dyn std::error::Error>> {
        let source = r##"
class Jq < Formula
  test do
    assert_equal "2\n", pipe_output("#{bin}/jq .bar", '{"foo":1, "bar":2}')
  end
end
"##;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 1);
        if let TestStatement::AssertEqual { actual, .. } = &program.statements[0] {
            assert!(matches!(actual, TestExpr::PipeOutput { .. }));
        } else {
            return Err("expected AssertEqual".into());
        }
        Ok(())
    }

    #[test]
    fn test_lower_pipe_output_three_args() -> Result<(), Box<dyn std::error::Error>> {
        let source = r##"
class Demo < Formula
  test do
    assert_match "ok", pipe_output("#{bin}/demo", "input", 0)
  end
end
"##;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 1);
        if let TestStatement::AssertMatch { actual, .. } = &program.statements[0] {
            assert!(matches!(
                actual,
                TestExpr::PipeOutput {
                    expected_status: Some(0),
                    ..
                }
            ));
        } else {
            return Err("expected AssertMatch".into());
        }
        Ok(())
    }

    #[test]
    fn test_lower_strip() -> Result<(), Box<dyn std::error::Error>> {
        let source = r##"
class Demo < Formula
  test do
    output = shell_output("#{bin}/demo").strip
    assert_equal "hello", output
  end
end
"##;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 2);
        if let TestStatement::Assign { value, .. } = &program.statements[0] {
            assert!(matches!(value, TestExpr::Strip(_)));
        } else {
            return Err("expected Assign".into());
        }
        Ok(())
    }

    #[test]
    fn test_lower_read_file() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  test do
    assert_equal "hello", (testpath/"output.txt").read.chomp
  end
end
"#;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 1);
        if let TestStatement::AssertEqual { actual, .. } = &program.statements[0] {
            assert!(
                matches!(actual, TestExpr::Chomp(inner) if matches!(inner.as_ref(), TestExpr::ReadFile(_)))
            );
        } else {
            return Err("expected AssertEqual".into());
        }
        Ok(())
    }

    #[test]
    fn test_lower_if_os_mac() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  test do
    if OS.mac?
      system bin/"demo", "--mac"
    else
      system bin/"demo", "--linux"
    end
  end
end
"#;

        let program = lower_test_do(source)?;

        // On macOS, only the then-branch should be emitted
        assert_eq!(program.statements.len(), 1);
        assert!(matches!(&program.statements[0], TestStatement::System(_)));
        Ok(())
    }

    #[test]
    fn test_lower_unless_os_mac_skips_body() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  test do
    system bin/"demo"
    unless OS.mac?
      system bin/"demo", "--linux-only"
    end
  end
end
"#;

        let program = lower_test_do(source)?;

        // unless OS.mac? body is skipped on macOS
        assert_eq!(program.statements.len(), 1);
        Ok(())
    }

    #[test]
    fn test_lower_pkgshare_path() -> Result<(), Box<dyn std::error::Error>> {
        let source = r#"
class Demo < Formula
  test do
    assert_path_exists pkgshare/"data.txt"
  end
end
"#;

        let program = lower_test_do(source)?;

        assert_eq!(program.statements.len(), 1);
        if let TestStatement::AssertPathExists {
            path: TestExpr::Path(path),
        } = &program.statements[0]
        {
            assert_eq!(path.base, TestPathBase::Pkgshare);
        } else {
            return Err("expected AssertPathExists with Pkgshare".into());
        }
        Ok(())
    }
}
