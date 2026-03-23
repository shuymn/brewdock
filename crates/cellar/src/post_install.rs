use std::path::PathBuf;

pub use brewdock_analysis::{
    Argument, ContentPart, PathBase, PathCondition, PathExpr, PathSegment, Program, SegmentPart,
    Statement, extract_post_install_block, lower_post_install, lower_post_install_tier2,
    validate_post_install,
};

use crate::error::CellarError;

mod context;
mod execute;
mod rollback;

use self::{
    execute::execute_statements,
    rollback::{collect_rollback_roots, restore_backups, run_with_rollback},
};

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

#[cfg(test)]
mod tests;
