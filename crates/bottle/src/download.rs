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
#[derive(Debug, Clone)]
pub struct HttpBottleDownloader {
    client: reqwest::Client,
    ghcr_base_url: String,
}

impl HttpBottleDownloader {
    /// Creates a downloader with a default HTTP client.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            ghcr_base_url: "https://ghcr.io".to_owned(),
        }
    }

    /// Creates a downloader with a custom client and GHCR base URL.
    #[must_use]
    pub const fn with_client_and_ghcr_base_url(
        client: reqwest::Client,
        ghcr_base_url: String,
    ) -> Self {
        Self {
            client,
            ghcr_base_url,
        }
    }

    /// Fetches an anonymous bearer token for GHCR.
    ///
    /// GHCR requires a token even for public images. The token endpoint
    /// returns a short-lived token scoped to the requested repository.
    async fn ghcr_token(&self, scope: &str) -> Result<String, BottleError> {
        let url = format!(
            "{}/token?service=ghcr.io&scope={scope}",
            self.ghcr_base_url.trim_end_matches('/')
        );
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
    fn ghcr_scope_with_base(url: &str, ghcr_base_url: &str) -> Option<String> {
        let prefix = format!("{}/v2/", ghcr_base_url.trim_end_matches('/'));
        let path = url.strip_prefix(&prefix)?;
        let blobs_pos = path.find("/blobs/")?;
        let repo = &path[..blobs_pos];
        Some(format!("repository:{repo}:pull"))
    }

    fn ghcr_scope(&self, url: &str) -> Option<String> {
        Self::ghcr_scope_with_base(url, &self.ghcr_base_url)
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

        let request = if let Some(scope) = self.ghcr_scope(url) {
            let token = self.ghcr_token(&scope).await?;
            self.client
                .get(url)
                .header("Authorization", format!("Bearer {token}"))
        } else {
            self.client.get(url)
        };

        let mut response = request.send().await?.error_for_status()?;
        let mut verifier = StreamVerifier::new(expected_sha256)?;
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
    use httpmock::{Method::GET, MockServer};

    use super::*;

    const HELLO_WORLD_SHA256: &str =
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    #[test]
    fn test_ghcr_scope_standard_url() {
        let url = "https://ghcr.io/v2/homebrew/core/jq/blobs/sha256:abc123";
        assert_eq!(
            HttpBottleDownloader::ghcr_scope_with_base(url, "https://ghcr.io"),
            Some("repository:homebrew/core/jq:pull".to_owned())
        );
    }

    #[test]
    fn test_ghcr_scope_nested_path() {
        let url = "https://ghcr.io/v2/homebrew/core/oniguruma/blobs/sha256:def456";
        assert_eq!(
            HttpBottleDownloader::ghcr_scope_with_base(url, "https://ghcr.io"),
            Some("repository:homebrew/core/oniguruma:pull".to_owned())
        );
    }

    #[test]
    fn test_ghcr_scope_non_ghcr_url() {
        let url = "https://example.com/bottles/jq.tar.gz";
        assert_eq!(
            HttpBottleDownloader::ghcr_scope_with_base(url, "https://ghcr.io"),
            None
        );
    }

    #[test]
    fn test_ghcr_scope_no_blobs_segment() {
        let url = "https://ghcr.io/v2/homebrew/core/jq/manifests/latest";
        assert_eq!(
            HttpBottleDownloader::ghcr_scope_with_base(url, "https://ghcr.io"),
            None
        );
    }

    #[tokio::test]
    async fn test_download_verified_success() -> Result<(), Box<dyn std::error::Error>> {
        let server = MockServer::start();
        let body = b"hello world";

        let bottle = server.mock(|when, then| {
            when.method(GET).path("/bottles/jq.tar.gz");
            then.status(200).body(body.as_slice());
        });

        let downloader = HttpBottleDownloader::new();
        let bytes = downloader
            .download_verified(&server.url("/bottles/jq.tar.gz"), HELLO_WORLD_SHA256)
            .await?;

        bottle.assert();
        assert_eq!(bytes, body);
        Ok(())
    }

    #[tokio::test]
    async fn test_download_verified_http_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/missing");
            then.status(500);
        });

        let downloader = HttpBottleDownloader::new();
        let result = downloader
            .download_verified(&server.url("/missing"), HELLO_WORLD_SHA256)
            .await;

        assert!(matches!(result, Err(BottleError::Download(_))));
    }

    #[tokio::test]
    async fn test_download_verified_checksum_mismatch() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/wrong");
            then.status(200).body("wrong body");
        });

        let downloader = HttpBottleDownloader::new();
        let result = downloader
            .download_verified(&server.url("/wrong"), HELLO_WORLD_SHA256)
            .await;

        assert!(matches!(result, Err(BottleError::ChecksumMismatch { .. })));
    }

    #[tokio::test]
    async fn test_download_verified_ghcr_token_flow() -> Result<(), Box<dyn std::error::Error>> {
        let server = MockServer::start();
        let token = server.mock(|when, then| {
            when.method(GET)
                .path("/token")
                .query_param("service", "ghcr.io")
                .query_param("scope", "repository:homebrew/core/jq:pull");
            then.status(200)
                .json_body(serde_json::json!({ "token": "test-token" }));
        });
        let blob = server.mock(|when, then| {
            when.method(GET)
                .path("/v2/homebrew/core/jq/blobs/sha256:abc123")
                .header("authorization", "Bearer test-token");
            then.status(200).body("hello world");
        });

        let downloader = HttpBottleDownloader::with_client_and_ghcr_base_url(
            reqwest::Client::new(),
            server.base_url(),
        );
        let bytes = downloader
            .download_verified(
                &server.url("/v2/homebrew/core/jq/blobs/sha256:abc123"),
                HELLO_WORLD_SHA256,
            )
            .await?;

        token.assert();
        blob.assert();
        assert_eq!(bytes, b"hello world");
        Ok(())
    }

    #[tokio::test]
    async fn test_download_verified_ghcr_token_missing() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/token")
                .query_param("service", "ghcr.io")
                .query_param("scope", "repository:homebrew/core/jq:pull");
            then.status(200).json_body(serde_json::json!({}));
        });

        let downloader = HttpBottleDownloader::with_client_and_ghcr_base_url(
            reqwest::Client::new(),
            server.base_url(),
        );
        let result = downloader
            .download_verified(
                &server.url("/v2/homebrew/core/jq/blobs/sha256:abc123"),
                HELLO_WORLD_SHA256,
            )
            .await;

        assert!(matches!(result, Err(BottleError::Auth(_))));
    }
}
