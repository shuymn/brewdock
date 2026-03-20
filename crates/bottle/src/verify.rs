use sha2::{Digest, Sha256};

use crate::error::{BottleError, Sha256Hex};

/// Verifies that data matches the expected SHA256 hex digest.
///
/// # Errors
///
/// Returns [`BottleError::ChecksumMismatch`] if the computed digest differs from `expected_hex`.
pub fn verify_sha256(data: &[u8], expected_hex: &str) -> Result<(), BottleError> {
    let mut v = StreamVerifier::new(expected_hex)?;
    v.update(data);
    v.finish()
}

/// Streaming SHA256 verifier that accumulates chunks incrementally.
///
/// Computes the hash over data received in chunks (e.g., during download)
/// and verifies the final digest against an expected value.
#[derive(Debug)]
pub struct StreamVerifier {
    hasher: Sha256,
    expected: Sha256Hex,
}

impl StreamVerifier {
    /// Creates a new verifier expecting the given hex digest.
    ///
    /// # Errors
    ///
    /// Returns [`BottleError::InvalidSha256`] if `expected_sha256` is not a valid digest.
    pub fn new(expected_sha256: &str) -> Result<Self, BottleError> {
        Ok(Self {
            hasher: Sha256::new(),
            expected: Sha256Hex::parse(expected_sha256)?,
        })
    }

    /// Feeds a chunk of data into the hasher.
    pub fn update(&mut self, chunk: &[u8]) {
        self.hasher.update(chunk);
    }

    /// Finalizes the hash and verifies against the expected digest.
    ///
    /// # Errors
    ///
    /// Returns [`BottleError::ChecksumMismatch`] if the computed digest differs from the expected value.
    pub fn finish(self) -> Result<(), BottleError> {
        let result = self.hasher.finalize();
        let actual = Sha256Hex::parse(&format!("{result:x}"))?;
        if actual != self.expected {
            return Err(BottleError::ChecksumMismatch {
                expected: self.expected,
                actual,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SHA256 of empty data.
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    // SHA256 of b"hello world".
    const HELLO_SHA256: &str = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    #[test]
    fn test_verify_sha256_valid_data() -> Result<(), BottleError> {
        verify_sha256(b"hello world", HELLO_SHA256)
    }

    #[test]
    fn test_verify_sha256_empty_data() -> Result<(), BottleError> {
        verify_sha256(b"", EMPTY_SHA256)
    }

    #[test]
    fn test_verify_sha256_mismatch_returns_checksum_error() {
        let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
        let result = verify_sha256(b"hello world", wrong);
        let Err(BottleError::ChecksumMismatch { expected, actual }) = result else {
            assert!(
                matches!(result, Err(BottleError::ChecksumMismatch { .. })),
                "expected ChecksumMismatch"
            );
            return;
        };
        assert_eq!(expected.as_str(), wrong);
        assert_eq!(actual.as_str(), HELLO_SHA256);
    }

    #[test]
    fn test_stream_verifier_single_chunk() -> Result<(), BottleError> {
        let mut v = StreamVerifier::new(HELLO_SHA256)?;
        v.update(b"hello world");
        v.finish()
    }

    #[test]
    fn test_stream_verifier_multiple_chunks() -> Result<(), BottleError> {
        let mut v = StreamVerifier::new(HELLO_SHA256)?;
        v.update(b"hello");
        v.update(b" ");
        v.update(b"world");
        v.finish()
    }

    #[test]
    fn test_stream_verifier_empty_data() -> Result<(), BottleError> {
        let v = StreamVerifier::new(EMPTY_SHA256)?;
        v.finish()
    }

    #[test]
    fn test_stream_verifier_mismatch() -> Result<(), BottleError> {
        let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
        let mut v = StreamVerifier::new(wrong)?;
        v.update(b"hello world");
        assert!(matches!(
            v.finish(),
            Err(BottleError::ChecksumMismatch { .. })
        ));
        Ok(())
    }

    #[test]
    fn test_stream_verifier_matches_one_shot() -> Result<(), BottleError> {
        let data = b"some arbitrary data for hashing";
        let hash = format!("{:x}", Sha256::digest(data));
        verify_sha256(data, &hash)?;

        let mut v = StreamVerifier::new(&hash)?;
        v.update(&data[..10]);
        v.update(&data[10..]);
        v.finish()
    }

    #[test]
    fn test_stream_verifier_rejects_invalid_expected_digest() {
        let result = StreamVerifier::new("short");
        assert!(matches!(result, Err(BottleError::InvalidSha256 { .. })));
    }
}
