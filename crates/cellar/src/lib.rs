#![warn(clippy::pedantic, clippy::nursery)]

//! Cellar materialization, receipt, linking, and state management for brewdock.

pub mod discover;
pub mod error;
pub(crate) mod fs;
pub mod link;
pub mod materialize;
pub mod post_install;
pub mod receipt;
pub mod relocate;
pub mod state;
pub mod test_do;

pub use discover::{InstalledKeg, discover_installed_kegs, find_installed_keg};
pub use error::CellarError;
pub use link::{link, unlink};
pub use materialize::{
    BottlePrefixTransaction, atomic_symlink_replace, install_bottle_etc_var, materialize,
};
pub use post_install::{
    PlatformContext, PostInstallContext, PostInstallTransaction, extract_post_install_block,
    lower_post_install, lower_post_install_tier2, run_post_install, validate_post_install,
};
pub use receipt::{
    InstallReason, InstallReceipt, ReceiptDependency, ReceiptSource, ReceiptSourceVersions,
    canonical_homebrew_arch, write_receipt,
};
pub use relocate::{RelocationManifest, RelocationScope, relocate_keg, relocate_keg_with_manifest};
pub use test_do::{TestDoContext, run_test_do};
