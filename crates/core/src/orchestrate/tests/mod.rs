use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use brewdock_bottle::{BottleError, extract_tar_gz};
use brewdock_cellar::{
    InstallReason, InstallReceipt, ReceiptSource, ReceiptSourceVersions, atomic_symlink_replace,
    write_receipt,
};
use brewdock_formula::{CellarType, MetadataStore};

use super::*;
use crate::testutil::{
    HOST_TAG, MockDownloader, MockRepo, PLAN_SHA, SHA_A, SHA_B, SHA_C, assert_installed,
    assert_not_installed, create_bottle_tar_gz, create_simple_bottle, make_formula,
    make_orchestrator, move_host_bottle_to_tag, setup_installed_keg,
};

mod install;
mod metadata;
mod post_install;
mod query;
mod source_fallback;
mod upgrade;

/// Sets the cellar type on the host bottle of a formula.
fn set_bottle_cellar(formula: &mut Formula, cellar: CellarType) {
    if let Some(ref mut stable) = formula.bottle.stable
        && let Some(file) = stable.files.get_mut(HOST_TAG)
    {
        file.cellar = cellar;
    }
}

pub(super) struct TrackingDownloader {
    data: HashMap<String, Vec<u8>>,
    delay: Duration,
    in_flight: Arc<AtomicUsize>,
    max_in_flight: Arc<AtomicUsize>,
}

impl TrackingDownloader {
    fn new(entries: Vec<(&str, Vec<u8>)>, delay: Duration) -> Self {
        Self {
            data: entries
                .into_iter()
                .map(|(checksum, bytes)| (checksum.to_owned(), bytes))
                .collect(),
            delay,
            in_flight: Arc::new(AtomicUsize::new(0)),
            max_in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }
}

impl BottleDownloader for TrackingDownloader {
    async fn download_verified(
        &self,
        _url: &str,
        expected_sha256: &str,
    ) -> Result<Vec<u8>, BottleError> {
        let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(current, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        self.data.get(expected_sha256).cloned().ok_or_else(|| {
            BottleError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("mock: no data for {expected_sha256}"),
            ))
        })
    }
}
