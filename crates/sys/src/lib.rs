#![warn(clippy::pedantic, clippy::nursery)]

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::clone_path;
