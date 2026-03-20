#![warn(clippy::pedantic, clippy::nursery)]

//! Bottle download, verification, and extraction for brewdock.

pub mod download;
pub mod error;
pub mod extract;
pub mod store;
pub mod verify;

pub use download::{BottleDownloader, HttpBottleDownloader};
pub use error::BottleError;
pub use extract::extract_tar_gz;
pub use store::BlobStore;
pub use verify::{StreamVerifier, verify_sha256};
