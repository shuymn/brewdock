#![warn(clippy::pedantic, clippy::nursery)]

//! Cellar materialization, receipt, linking, and state management for brewdock.

pub mod discover;
pub mod error;
pub mod link;
pub mod materialize;
pub mod post_install;
pub mod receipt;
pub mod relocate;
pub mod state;
pub(crate) mod util;

pub use discover::{InstalledKeg, discover_installed_kegs, find_installed_keg};
pub use error::CellarError;
pub use link::{link, unlink};
pub use materialize::{atomic_symlink_replace, materialize};
pub use post_install::{
    PostInstallContext, PostInstallTransaction, extract_post_install_block, run_post_install,
    validate_post_install,
};
pub use receipt::{
    InstallReason, InstallReceipt, ReceiptDependency, ReceiptSource, ReceiptSourceVersions,
    write_receipt,
};
pub use relocate::relocate_keg;
