#![warn(clippy::pedantic, clippy::nursery)]

//! Static analysis of Homebrew formula `post_install` blocks.
//!
//! This crate parses Ruby formula source using `ruby-prism`, lowers
//! allowlisted AST shapes into an internal [`post_install::Program`]
//! representation, and applies schema normalization — all without
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

pub use error::AnalysisError;
pub use post_install::{
    Argument, ContentPart, PathBase, PathCondition, PathExpr, PathSegment, Program, SegmentPart,
    Statement, extract_post_install_block, lower_post_install, lower_post_install_tier2,
    validate_post_install,
};
