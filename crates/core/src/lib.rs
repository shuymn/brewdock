#![warn(clippy::pedantic, clippy::nursery, clippy::cargo)]

//! Core types and orchestration for brewdock.

pub mod error;
pub mod layout;
pub mod platform;

pub use error::BrewdockError;
pub use layout::Layout;
pub use platform::HostTag;
