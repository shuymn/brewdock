use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use brewdock_analysis::{
    TestArg, TestExpr, TestPathBase, TestPathExpr, TestProgram, TestStatement, TestStringExpr,
    TestStringPart, lower_test_do,
};
use tempfile::TempDir;

use crate::{error::CellarError, fs::normalize_absolute_path};

/// Execution environment for the restricted `test do` DSL.
#[derive(Debug)]
pub struct TestDoContext {
    formula_name: String,
    formula_version: String,
    keg_path: PathBuf,
    testdir: TempDir,
    variables: BTreeMap<String, TestValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TestValue {
    String(String),
    Path(PathBuf),
}

impl TestDoContext {
    /// Creates a new context rooted at a temporary `testpath`.
    ///
    /// # Errors
    ///
    /// Returns [`CellarError::Io`] when the temporary directory cannot be created.
    pub fn new(
        keg_path: &Path,
        formula_name: &str,
        formula_version: &str,
    ) -> Result<Self, CellarError> {
        Ok(Self {
            formula_name: formula_name.to_owned(),
            formula_version: formula_version.to_owned(),
            keg_path: keg_path.to_path_buf(),
            testdir: tempfile::tempdir()?,
            variables: BTreeMap::new(),
        })
    }

    /// Returns the temporary `testpath`.
    #[must_use]
    pub fn testpath(&self) -> &Path {
        self.testdir.path()
    }

    fn resolve_path(&self, expr: &TestPathExpr) -> Result<PathBuf, CellarError> {
        let mut raw = match expr.base {
            TestPathBase::Testpath => self.testdir.path().to_path_buf(),
            TestPathBase::Prefix => self.keg_path.clone(),
            TestPathBase::Bin => self.keg_path.join("bin"),
            TestPathBase::Include => self.keg_path.join("include"),
            TestPathBase::Lib => self.keg_path.join("lib"),
            TestPathBase::Libexec => self.keg_path.join("libexec"),
            TestPathBase::Pkgshare => self.keg_path.join("share").join(&self.formula_name),
            TestPathBase::Sbin => self.keg_path.join("sbin"),
            TestPathBase::Share => self.keg_path.join("share"),
        };
        for segment in &expr.segments {
            raw.push(segment);
        }
        let normalized =
            normalize_absolute_path(&raw).ok_or_else(|| CellarError::UnsupportedTestDoSyntax {
                message: format!("path escapes allowed roots: {}", raw.display()),
            })?;
        if normalized.starts_with(self.testdir.path()) || normalized.starts_with(&self.keg_path) {
            return Ok(normalized);
        }
        Err(CellarError::UnsupportedTestDoSyntax {
            message: format!("path escapes allowed roots: {}", normalized.display()),
        })
    }
}

/// Executes a lowered or source `test do` program.
///
/// # Errors
///
/// Returns [`CellarError::Analysis`] for lowering failures,
/// [`CellarError::TestDoCommandFailed`] for command failures, and
/// [`CellarError::TestDoAssertionFailed`] for assertion failures.
pub fn run_test_do(source: &str, context: &mut TestDoContext) -> Result<(), CellarError> {
    let program = lower_test_do(source)?;
    execute_test_program(&program, context)
}

fn execute_test_program(
    program: &TestProgram,
    context: &mut TestDoContext,
) -> Result<(), CellarError> {
    for statement in &program.statements {
        match statement {
            TestStatement::Assign { variable, value } => {
                let resolved = eval_expr(value, context)?;
                context.variables.insert(variable.clone(), resolved);
            }
            TestStatement::WriteFile { path, content } => {
                let path = context.resolve_path(path)?;
                if !path.starts_with(context.testdir.path()) {
                    return Err(CellarError::UnsupportedTestDoSyntax {
                        message: format!(
                            "writes outside testpath are not allowed: {}",
                            path.display()
                        ),
                    });
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, eval_expr_string(content, context)?)?;
            }
            TestStatement::System(arguments) => run_system(arguments, context)?,
            TestStatement::AssertMatch { expected, actual } => {
                let expected = eval_expr_string(expected, context)?;
                let actual = eval_expr_string(actual, context)?;
                if actual.contains(&expected) {
                    continue;
                }
                return Err(CellarError::TestDoAssertionFailed {
                    message: format!("expected output to contain {expected:?}, got {actual:?}"),
                });
            }
            TestStatement::AssertEqual { expected, actual } => {
                let expected = eval_expr_string(expected, context)?;
                let actual = eval_expr_string(actual, context)?;
                if expected == actual {
                    continue;
                }
                return Err(CellarError::TestDoAssertionFailed {
                    message: format!("expected {expected:?}, got {actual:?}"),
                });
            }
            TestStatement::Mkpath(path) => {
                let resolved = context.resolve_path(path)?;
                if !resolved.starts_with(context.testdir.path()) {
                    return Err(CellarError::UnsupportedTestDoSyntax {
                        message: format!(
                            "mkpath outside testpath not allowed: {}",
                            resolved.display()
                        ),
                    });
                }
                std::fs::create_dir_all(resolved)?;
            }
            TestStatement::Touch(expr) => {
                let path = match eval_expr(expr, context)? {
                    TestValue::Path(p) => p,
                    TestValue::String(s) => context.testdir.path().join(s),
                };
                if !path.starts_with(context.testdir.path()) {
                    return Err(CellarError::UnsupportedTestDoSyntax {
                        message: format!("touch outside testpath not allowed: {}", path.display()),
                    });
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .write(true)
                    .open(path)?;
            }
            TestStatement::RefuteMatch { expected, actual } => {
                let expected = eval_expr_string(expected, context)?;
                let actual = eval_expr_string(actual, context)?;
                if actual.contains(&expected) {
                    return Err(CellarError::TestDoAssertionFailed {
                        message: format!(
                            "expected output NOT to contain {expected:?}, got {actual:?}"
                        ),
                    });
                }
            }
            TestStatement::AssertPathExists { path } => {
                let resolved = resolve_expr_to_path(path, context)?;
                if !resolved.exists() {
                    return Err(CellarError::TestDoAssertionFailed {
                        message: format!("expected path to exist: {}", resolved.display()),
                    });
                }
            }
            TestStatement::RefutePathExists { path } => {
                let resolved = resolve_expr_to_path(path, context)?;
                if resolved.exists() {
                    return Err(CellarError::TestDoAssertionFailed {
                        message: format!("expected path NOT to exist: {}", resolved.display()),
                    });
                }
            }
        }
    }
    Ok(())
}

fn resolve_expr_to_path(expr: &TestExpr, context: &TestDoContext) -> Result<PathBuf, CellarError> {
    let path = match eval_expr(expr, context)? {
        TestValue::Path(p) => p,
        TestValue::String(s) => context.testdir.path().join(s),
    };
    if path.starts_with(context.testdir.path()) || path.starts_with(&context.keg_path) {
        return Ok(path);
    }
    Err(CellarError::UnsupportedTestDoSyntax {
        message: format!("path escapes allowed roots: {}", path.display()),
    })
}

fn eval_expr(expr: &TestExpr, context: &TestDoContext) -> Result<TestValue, CellarError> {
    match expr {
        TestExpr::Path(path) => Ok(TestValue::Path(context.resolve_path(path)?)),
        _ => Ok(TestValue::String(eval_expr_string(expr, context)?)),
    }
}

fn eval_expr_string(expr: &TestExpr, context: &TestDoContext) -> Result<String, CellarError> {
    match expr {
        TestExpr::String(value) => resolve_string(value, context),
        TestExpr::Path(path) => Ok(context.resolve_path(path)?.to_string_lossy().into_owned()),
        TestExpr::VersionString => Ok(context.formula_version.clone()),
        TestExpr::Variable(name) => match context.variables.get(name) {
            Some(TestValue::String(value)) => Ok(value.clone()),
            Some(TestValue::Path(path)) => Ok(path.to_string_lossy().into_owned()),
            None => Err(CellarError::UnsupportedTestDoSyntax {
                message: format!("unknown test variable: {name}"),
            }),
        },
        TestExpr::ShellOutput {
            command,
            expected_status,
        } => run_shell_output(command, *expected_status, context),
        TestExpr::PipeOutput {
            command,
            stdin,
            expected_status,
        } => run_pipe_output(command, stdin, *expected_status, context),
        TestExpr::Chomp(inner) => Ok(eval_expr_string(inner, context)?
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .to_owned()),
        TestExpr::Strip(inner) => Ok(eval_expr_string(inner, context)?.trim().to_owned()),
        TestExpr::ReadFile(path) => {
            let resolved = context.resolve_path(path)?;
            Ok(std::fs::read_to_string(resolved)?)
        }
    }
}

fn resolve_string(value: &TestStringExpr, context: &TestDoContext) -> Result<String, CellarError> {
    let mut resolved = String::new();
    for part in &value.parts {
        match part {
            TestStringPart::Literal(text) => resolved.push_str(text),
            TestStringPart::Path(path) => {
                resolved.push_str(&context.resolve_path(path)?.to_string_lossy());
            }
            TestStringPart::VersionString => resolved.push_str(&context.formula_version),
        }
    }
    Ok(resolved)
}

fn check_output(
    output: &std::process::Output,
    command: &str,
    expected_status: Option<i32>,
) -> Result<String, CellarError> {
    let expected_status = expected_status.unwrap_or(0);
    let Some(actual_status) = output.status.code() else {
        return Err(CellarError::TestDoCommandFailed {
            message: format!("command terminated by signal: {command}"),
        });
    };
    if actual_status != expected_status {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(CellarError::TestDoCommandFailed {
            message: if stderr.is_empty() {
                format!("expected exit status {expected_status}, got {actual_status}")
            } else {
                stderr
            },
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_shell_output(
    command: &TestStringExpr,
    expected_status: Option<i32>,
    context: &TestDoContext,
) -> Result<String, CellarError> {
    let command = resolve_string(command, context)?;
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(&command)
        .current_dir(context.testdir.path())
        .output()?;
    check_output(&output, &command, expected_status)
}

fn run_pipe_output(
    command: &TestStringExpr,
    stdin_expr: &TestExpr,
    expected_status: Option<i32>,
    context: &TestDoContext,
) -> Result<String, CellarError> {
    let command = resolve_string(command, context)?;
    let stdin_data = eval_expr_string(stdin_expr, context)?;
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(&command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .current_dir(context.testdir.path())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(stdin_data.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    check_output(&output, &command, expected_status)
}

fn run_system(arguments: &[TestArg], context: &TestDoContext) -> Result<(), CellarError> {
    let mut argv = arguments
        .iter()
        .map(|arg| resolve_arg(arg, context))
        .collect::<Result<Vec<_>, _>>()?;
    if argv.is_empty() {
        return Err(CellarError::UnsupportedTestDoSyntax {
            message: "system expects at least one argument".to_owned(),
        });
    }
    let program = argv.remove(0);
    let output = Command::new(&program)
        .args(&argv)
        .current_dir(context.testdir.path())
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(CellarError::TestDoCommandFailed {
        message: if stderr.is_empty() {
            format!("exit status {}", output.status)
        } else {
            stderr
        },
    })
}

fn resolve_arg(arg: &TestArg, context: &TestDoContext) -> Result<OsString, CellarError> {
    match arg {
        TestArg::Path(path) => Ok(context.resolve_path(path)?.into_os_string()),
        TestArg::String(value) => Ok(OsString::from(resolve_string(value, context)?)),
        TestArg::Variable(name) => match context.variables.get(name) {
            Some(TestValue::String(value)) => Ok(OsString::from(value)),
            Some(TestValue::Path(path)) => Ok(path.clone().into_os_string()),
            None => Err(CellarError::UnsupportedTestDoSyntax {
                message: format!("unknown test variable: {name}"),
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_executable(path: &Path, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, contents)?;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms)?;
        Ok(())
    }

    #[test]
    fn test_run_test_do_shfmt_subset() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/shfmt/3.10.0");
        std::fs::create_dir_all(keg.join("bin"))?;
        write_executable(
            &keg.join("bin/shfmt"),
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf '3.10.0\\n'\nelse\n  cat \"$1\"\nfi\n",
        )?;

        let source = r##"
class Shfmt < Formula
  test do
    assert_match version.to_s, shell_output("#{bin}/shfmt --version")
    (testpath/"test").write "\t\techo foo"
    system bin/"shfmt", testpath/"test"
  end
end
"##;

        let mut context = TestDoContext::new(&keg, "shfmt", "3.10.0")?;
        run_test_do(source, &mut context)?;
        assert_eq!(
            std::fs::read_to_string(context.testpath().join("test"))?,
            "\t\techo foo"
        );
        Ok(())
    }

    #[test]
    fn test_run_test_do_supports_chomp_and_variable() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/hello/1.0");
        std::fs::create_dir_all(keg.join("bin"))?;
        write_executable(
            &keg.join("bin/hello"),
            "#!/bin/sh\nprintf '%s\\n' \"${1#--greeting=}\"\n",
        )?;

        let source = r##"
class Hello < Formula
  test do
    output = shell_output("#{bin}/hello --greeting=brew").chomp
    assert_equal "brew", output
  end
end
"##;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        run_test_do(source, &mut context)?;
        Ok(())
    }

    #[test]
    fn test_run_test_do_rejects_writes_outside_testpath() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;

        let source = r#"
class Demo < Formula
  test do
    (prefix/"oops").write "bad"
  end
end
"#;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        let result = run_test_do(source, &mut context);
        assert!(matches!(
            result,
            Err(CellarError::UnsupportedTestDoSyntax { .. })
        ));
        Ok(())
    }

    #[test]
    fn test_run_test_do_assertion_failure() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(keg.join("bin"))?;
        write_executable(&keg.join("bin/demo"), "#!/bin/sh\nprintf 'hello\\n'\n")?;

        let source = r##"
class Demo < Formula
  test do
    assert_equal "goodbye", shell_output("#{bin}/demo").chomp
  end
end
"##;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        let result = run_test_do(source, &mut context);
        assert!(matches!(
            result,
            Err(CellarError::TestDoAssertionFailed { .. })
        ));
        Ok(())
    }

    #[test]
    fn test_run_refute_match_passes() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(keg.join("bin"))?;
        write_executable(&keg.join("bin/demo"), "#!/bin/sh\nprintf 'all good\\n'\n")?;

        let source = r##"
class Demo < Formula
  test do
    refute_match "error", shell_output("#{bin}/demo").chomp
  end
end
"##;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        run_test_do(source, &mut context)?;
        Ok(())
    }

    #[test]
    fn test_run_refute_match_fails() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(keg.join("bin"))?;
        write_executable(
            &keg.join("bin/demo"),
            "#!/bin/sh\nprintf 'error occurred\\n'\n",
        )?;

        let source = r##"
class Demo < Formula
  test do
    refute_match "error", shell_output("#{bin}/demo").chomp
  end
end
"##;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        let result = run_test_do(source, &mut context);
        assert!(matches!(
            result,
            Err(CellarError::TestDoAssertionFailed { .. })
        ));
        Ok(())
    }

    #[test]
    fn test_run_mkpath_creates_directory() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;

        let source = r#"
class Demo < Formula
  test do
    (testpath/"sub/dir").mkpath
  end
end
"#;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        run_test_do(source, &mut context)?;
        assert!(context.testpath().join("sub/dir").is_dir());
        Ok(())
    }

    #[test]
    fn test_run_touch_creates_file() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;

        let source = r#"
class Demo < Formula
  test do
    touch testpath/"foo_file"
  end
end
"#;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        run_test_do(source, &mut context)?;
        assert!(context.testpath().join("foo_file").exists());
        Ok(())
    }

    #[test]
    fn test_run_mkpath_rejects_outside_testpath() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;

        let source = r#"
class Demo < Formula
  test do
    (prefix/"sneaky").mkpath
  end
end
"#;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        let result = run_test_do(source, &mut context);
        assert!(matches!(
            result,
            Err(CellarError::UnsupportedTestDoSyntax { .. })
        ));
        Ok(())
    }

    #[test]
    fn test_run_assert_path_exists_passes() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(keg.join("bin"))?;

        let source = r#"
class Demo < Formula
  test do
    (testpath/"hello.txt").write "hi"
    assert_path_exists testpath/"hello.txt"
  end
end
"#;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        run_test_do(source, &mut context)?;
        Ok(())
    }

    #[test]
    fn test_run_assert_path_exists_fails() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;

        let source = r#"
class Demo < Formula
  test do
    assert_path_exists testpath/"nonexistent"
  end
end
"#;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        let result = run_test_do(source, &mut context);
        assert!(matches!(
            result,
            Err(CellarError::TestDoAssertionFailed { .. })
        ));
        Ok(())
    }

    #[test]
    fn test_run_refute_path_exists_passes() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;

        let source = r#"
class Demo < Formula
  test do
    refute_path_exists testpath/"nonexistent"
  end
end
"#;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        run_test_do(source, &mut context)?;
        Ok(())
    }

    #[test]
    fn test_run_refute_path_exists_fails() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;

        let source = r#"
class Demo < Formula
  test do
    (testpath/"exists.txt").write "data"
    refute_path_exists testpath/"exists.txt"
  end
end
"#;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        let result = run_test_do(source, &mut context);
        assert!(matches!(
            result,
            Err(CellarError::TestDoAssertionFailed { .. })
        ));
        Ok(())
    }

    #[test]
    fn test_run_pipe_output_basic() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(keg.join("bin"))?;

        let source = r#"
class Demo < Formula
  test do
    assert_match "bar", pipe_output("cat", "foobar")
  end
end
"#;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        run_test_do(source, &mut context)?;
        Ok(())
    }

    #[test]
    fn test_run_pipe_output_with_status() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(keg.join("bin"))?;

        let source = r#"
class Demo < Formula
  test do
    assert_match "hello", pipe_output("cat", "hello world", 0)
  end
end
"#;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        run_test_do(source, &mut context)?;
        Ok(())
    }

    #[test]
    fn test_run_strip() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(keg.join("bin"))?;
        write_executable(&keg.join("bin/demo"), "#!/bin/sh\nprintf '  hello  \\n'\n")?;

        let source = r##"
class Demo < Formula
  test do
    output = shell_output("#{bin}/demo").strip
    assert_equal "hello", output
  end
end
"##;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        run_test_do(source, &mut context)?;
        Ok(())
    }

    #[test]
    fn test_run_read_file() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(&keg)?;

        let source = r#"
class Demo < Formula
  test do
    (testpath/"hello.txt").write "world"
    assert_equal "world", (testpath/"hello.txt").read
  end
end
"#;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        run_test_do(source, &mut context)?;
        Ok(())
    }

    #[test]
    fn test_run_if_os_mac() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(keg.join("bin"))?;
        write_executable(&keg.join("bin/demo"), "#!/bin/sh\nprintf 'mac\\n'\n")?;

        let source = r##"
class Demo < Formula
  test do
    if OS.mac?
      assert_match "mac", shell_output("#{bin}/demo")
    end
  end
end
"##;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        run_test_do(source, &mut context)?;
        Ok(())
    }

    #[test]
    fn test_run_pkgshare_path() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let keg = dir.path().join("Cellar/demo/1.0");
        std::fs::create_dir_all(keg.join("share/demo"))?;
        std::fs::write(keg.join("share/demo/data.txt"), "content")?;

        let source = r#"
class Demo < Formula
  test do
    assert_path_exists pkgshare/"data.txt"
  end
end
"#;

        let mut context = TestDoContext::new(&keg, "demo", "1.0")?;
        run_test_do(source, &mut context)?;
        Ok(())
    }
}
