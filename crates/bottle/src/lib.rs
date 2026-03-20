#![warn(clippy::pedantic, clippy::nursery)]

//! Bottle download, verification, and extraction for brewdock.

mod download;
pub mod error;
mod extract;
mod store;
mod verify;

pub use download::{BottleDownloader, HttpBottleDownloader};
pub use error::{BottleError, Sha256Hex};
pub use extract::extract_tar_gz;
pub use store::BlobStore;
pub use verify::{StreamVerifier, verify_sha256};
