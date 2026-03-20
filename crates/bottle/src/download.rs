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
///
/// Handles GHCR (GitHub Container Registry) authentication transparently.
/// When a URL matches `ghcr.io`, an anonymous bearer token is fetched
/// automatically before downloading the blob.
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

    /// Fetches an anonymous bearer token for GHCR.
    ///
    /// GHCR requires a token even for public images. The token endpoint
    /// returns a short-lived token scoped to the requested repository.
    async fn ghcr_token(&self, scope: &str) -> Result<String, BottleError> {
        let url = format!("https://ghcr.io/token?service=ghcr.io&scope={scope}");
        let response = self.client.get(&url).send().await?.error_for_status()?;
        let body: serde_json::Value = response.json().await?;
        body["token"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| BottleError::Auth("no token in GHCR response".to_owned()))
    }

    /// Extracts the GHCR repository scope from a blob URL.
    ///
    /// `https://ghcr.io/v2/homebrew/core/jq/blobs/sha256:...`
    /// → `repository:homebrew/core/jq:pull`
    fn ghcr_scope(url: &str) -> Option<String> {
        let path = url.strip_prefix("https://ghcr.io/v2/")?;
        let blobs_pos = path.find("/blobs/")?;
        let repo = &path[..blobs_pos];
        Some(format!("repository:{repo}:pull"))
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

        let request = if let Some(scope) = Self::ghcr_scope(url) {
            let token = self.ghcr_token(&scope).await?;
            self.client
                .get(url)
                .header("Authorization", format!("Bearer {token}"))
        } else {
            self.client.get(url)
        };

        let mut response = request.send().await?.error_for_status()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ghcr_scope_standard_url() {
        let url = "https://ghcr.io/v2/homebrew/core/jq/blobs/sha256:abc123";
        assert_eq!(
            HttpBottleDownloader::ghcr_scope(url),
            Some("repository:homebrew/core/jq:pull".to_owned())
        );
    }

    #[test]
    fn test_ghcr_scope_nested_path() {
        let url = "https://ghcr.io/v2/homebrew/core/oniguruma/blobs/sha256:def456";
        assert_eq!(
            HttpBottleDownloader::ghcr_scope(url),
            Some("repository:homebrew/core/oniguruma:pull".to_owned())
        );
    }

    #[test]
    fn test_ghcr_scope_non_ghcr_url() {
        let url = "https://example.com/bottles/jq.tar.gz";
        assert_eq!(HttpBottleDownloader::ghcr_scope(url), None);
    }

    #[test]
    fn test_ghcr_scope_no_blobs_segment() {
        let url = "https://ghcr.io/v2/homebrew/core/jq/manifests/latest";
        assert_eq!(HttpBottleDownloader::ghcr_scope(url), None);
    }
}
