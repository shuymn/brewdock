#![warn(clippy::pedantic, clippy::nursery)]

//! Static analysis of Homebrew formula `post_install` and `test do` blocks.
//!
//! This crate parses Ruby formula source using `ruby-prism`, lowers
//! allowlisted AST shapes into an internal [`post_install::Program`]
//! representation, lowers a restricted `test do` subset into an internal
//! [`test_do::TestProgram`], and applies schema normalization — all without
//! executing any Ruby code or touching the filesystem.
//!
//! # Usage
//!
//! ```no_run
//! # #[expect(clippy::unwrap_used)]
//! # fn main() {
//! let source = std::fs::read_to_string("formula.rb").unwrap();
//! brewdock_analysis::validate_post_install(&source, "1.0").unwrap();
//! # }
//! ```

pub mod error;
pub mod post_install;
pub mod test_do;

pub use error::AnalysisError;
pub use post_install::{
    Argument, ContentPart, PathBase, PathCondition, PathExpr, PathSegment, PostInstallAnalysis,
    PostInstallFeatures, Program, SegmentPart, Statement, analyze_post_install_all,
    extract_post_install_block, lower_post_install, lower_post_install_tier2,
    validate_post_install,
};
pub use test_do::{
    TestArg, TestDoAnalysis, TestDoFeatures, TestExpr, TestPathBase, TestPathExpr, TestProgram,
    TestStatement, TestStringExpr, TestStringPart, analyze_test_do, analyze_test_do_all,
    extract_test_do_block, lower_test_do, validate_test_do,
};
