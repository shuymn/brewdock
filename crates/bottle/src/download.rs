use crate::{error::BottleError, verify::StreamVerifier};

/// Repository for downloading bottle files.
///
/// Implemented by [`HttpBottleDownloader`] for production use.
/// Use generics (not trait objects) to consume this trait.
pub trait BottleDownloader: Send + Sync {
    /// Downloads a bottle from the given URL, stream-verifying its SHA256 checksum.
    ///
    /// The hash is computed incrementally as chunks arrive.
    ///
    /// # Errors
    ///
    /// Returns [`BottleError::Download`] on HTTP failure.
    /// Returns [`BottleError::ChecksumMismatch`] if the digest does not match.
    fn download_verified(
        &self,
        url: &str,
        expected_sha256: &str,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, BottleError>> + Send;
}

/// HTTP-based bottle downloader using reqwest.
#[derive(Debug)]
pub struct HttpBottleDownloader {
    client: reqwest::Client,
}

impl HttpBottleDownloader {
    /// Creates a downloader with a default HTTP client.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for HttpBottleDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl BottleDownloader for HttpBottleDownloader {
    async fn download_verified(
        &self,
        url: &str,
        expected_sha256: &str,
    ) -> Result<Vec<u8>, BottleError> {
        tracing::debug!(url, "downloading bottle");

        let mut response = self.client.get(url).send().await?.error_for_status()?;
        let mut verifier = StreamVerifier::new(expected_sha256);
        let capacity = response
            .content_length()
            .unwrap_or(0)
            .try_into()
            .unwrap_or(0usize);
        let mut data = Vec::with_capacity(capacity);

        while let Some(chunk) = response.chunk().await? {
            verifier.update(&chunk);
            data.extend_from_slice(&chunk);
        }

        verifier.finish()?;
        tracing::debug!(bytes = data.len(), "download verified");
        Ok(data)
    }
}
