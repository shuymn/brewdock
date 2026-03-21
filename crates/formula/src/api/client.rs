use crate::{error::FormulaError, types::Formula};

const DEFAULT_API_BASE_URL: &str = "https://formulae.brew.sh/api";
const DEFAULT_CORE_RAW_BASE_URL: &str =
    "https://raw.githubusercontent.com/Homebrew/homebrew-core/HEAD";

/// Repository for fetching formula metadata.
///
/// Implemented by [`HttpFormulaRepository`] for production use.
/// Use generics (not trait objects) to consume this trait.
pub trait FormulaRepository: Send + Sync {
    /// Fetches a single formula by name.
    ///
    /// # Errors
    ///
    /// Returns [`FormulaError::NotFound`] if the formula does not exist.
    /// Returns [`FormulaError::Network`] on HTTP failure.
    fn get_formula(
        &self,
        name: &str,
    ) -> impl std::future::Future<Output = Result<Formula, FormulaError>> + Send;

    /// Fetches the full formula index.
    ///
    /// # Errors
    ///
    /// Returns [`FormulaError::Network`] on HTTP failure.
    fn get_all_formulae(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<Formula>, FormulaError>> + Send;

    /// Fetches the raw Ruby formula source from `homebrew/core`.
    ///
    /// # Errors
    ///
    /// Returns [`FormulaError::Network`] on HTTP failure.
    fn get_ruby_source(
        &self,
        ruby_source_path: &str,
    ) -> impl std::future::Future<Output = Result<String, FormulaError>> + Send;
}

/// HTTP-based formula repository using the Homebrew JSON API.
#[derive(Debug, Clone)]
pub struct HttpFormulaRepository {
    client: reqwest::Client,
    base_url: String,
    core_raw_base_url: String,
}

impl HttpFormulaRepository {
    /// Creates a repository pointing at the default Homebrew API.
    #[must_use]
    pub fn new() -> Self {
        Self::with_urls(
            DEFAULT_API_BASE_URL.to_owned(),
            DEFAULT_CORE_RAW_BASE_URL.to_owned(),
        )
    }

    /// Creates a repository with a custom base URL.
    #[must_use]
    pub fn with_base_url(base_url: String) -> Self {
        Self::with_urls(base_url, DEFAULT_CORE_RAW_BASE_URL.to_owned())
    }

    /// Creates a repository with custom API and raw-source base URLs.
    #[must_use]
    pub fn with_urls(base_url: String, core_raw_base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
            core_raw_base_url,
        }
    }
}

impl Default for HttpFormulaRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl FormulaRepository for HttpFormulaRepository {
    async fn get_formula(&self, name: &str) -> Result<Formula, FormulaError> {
        let url = format!("{}/formula/{name}.json", self.base_url);
        let response = self.client.get(&url).send().await?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(FormulaError::NotFound {
                name: name.to_owned(),
            });
        }

        let response = response.error_for_status()?;
        let formula: Formula = response.json().await?;
        Ok(formula)
    }

    async fn get_all_formulae(&self) -> Result<Vec<Formula>, FormulaError> {
        let url = format!("{}/formula.json", self.base_url);
        let response = self.client.get(&url).send().await?;
        let response = response.error_for_status()?;
        let formulae: Vec<Formula> = response.json().await?;
        Ok(formulae)
    }

    async fn get_ruby_source(&self, ruby_source_path: &str) -> Result<String, FormulaError> {
        let path = validate_ruby_source_path(ruby_source_path)?;
        let url = format!("{}/{}", self.core_raw_base_url, path);
        let response = self.client.get(&url).send().await?;
        let response = response.error_for_status()?;
        response.text().await.map_err(FormulaError::from)
    }
}

fn validate_ruby_source_path(ruby_source_path: &str) -> Result<&str, FormulaError> {
    if ruby_source_path.is_empty()
        || ruby_source_path.starts_with('/')
        || !ruby_source_path.starts_with("Formula/")
        || ruby_source_path.contains('%')
    {
        return Err(FormulaError::InvalidRubySourcePath {
            path: ruby_source_path.to_owned(),
        });
    }

    for segment in ruby_source_path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(FormulaError::InvalidRubySourcePath {
                path: ruby_source_path.to_owned(),
            });
        }
    }

    Ok(ruby_source_path)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    use httpmock::{Method::GET, MockServer};

    use super::*;

    const JQ_FIXTURE: &str = include_str!("../../tests/fixtures/formula/jq.json");
    const SEMGREP_FIXTURE: &str = include_str!("../../tests/fixtures/formula/semgrep.json");
    const ZSH_COMPLETIONS_FIXTURE: &str =
        include_str!("../../tests/fixtures/formula/zsh-completions.json");

    fn make_repo(server: &MockServer) -> HttpFormulaRepository {
        HttpFormulaRepository::with_urls(server.base_url(), server.base_url())
    }

    type CapturedPath = Arc<Mutex<Option<String>>>;

    fn spawn_capture_server() -> Result<(String, CapturedPath), std::io::Error> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let captured_path = Arc::new(Mutex::new(None));
        let captured_path_for_thread = Arc::clone(&captured_path);

        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buffer = [0_u8; 4096];
                        let Ok(bytes_read) = stream.read(&mut buffer) else {
                            break;
                        };
                        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
                        let first_line = request.lines().next().unwrap_or("");
                        let path = first_line
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("")
                            .to_owned();

                        if let Ok(mut guard) = captured_path_for_thread.lock() {
                            *guard = Some(path.clone());
                        }

                        let body = if path == "/root/secret.rb" {
                            "safe"
                        } else if path == "/secret.rb" {
                            "secret"
                        } else {
                            "missing"
                        };
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body,
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        break;
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok((format!("http://{addr}"), captured_path))
    }

    #[tokio::test]
    async fn test_get_formula_success() -> Result<(), Box<dyn std::error::Error>> {
        let server = MockServer::start();
        let formula = server.mock(|when, then| {
            when.method(GET).path("/formula/jq.json");
            then.status(200)
                .header("content-type", "application/json")
                .body(JQ_FIXTURE);
        });

        let repo = make_repo(&server);
        let result = repo.get_formula("jq").await?;

        formula.assert();
        assert_eq!(result.name, "jq");
        assert_eq!(result.versions.stable, "1.8.1");
        Ok(())
    }

    #[tokio::test]
    async fn test_get_formula_not_found() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/formula/missing.json");
            then.status(404);
        });

        let repo = make_repo(&server);
        let result = repo.get_formula("missing").await;

        assert!(matches!(result, Err(FormulaError::NotFound { .. })));
    }

    #[tokio::test]
    async fn test_get_formula_non_404_http_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/formula/jq.json");
            then.status(500);
        });

        let repo = make_repo(&server);
        let result = repo.get_formula("jq").await;

        assert!(matches!(result, Err(FormulaError::Network(_))));
    }

    #[tokio::test]
    async fn test_get_formula_invalid_json() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/formula/jq.json");
            then.status(200)
                .header("content-type", "application/json")
                .body("{");
        });

        let repo = make_repo(&server);
        let result = repo.get_formula("jq").await;

        assert!(matches!(result, Err(FormulaError::Network(_))));
    }

    #[tokio::test]
    async fn test_get_formula_parses_live_uses_from_macos_shape()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = MockServer::start();
        let formula = server.mock(|when, then| {
            when.method(GET).path("/formula/zsh-completions.json");
            then.status(200)
                .header("content-type", "application/json")
                .body(ZSH_COMPLETIONS_FIXTURE);
        });

        let repo = make_repo(&server);
        let result = repo.get_formula("zsh-completions").await?;

        formula.assert();
        assert_eq!(result.name, "zsh-completions");
        assert_eq!(result.uses_from_macos.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_all_formulae_success() -> Result<(), Box<dyn std::error::Error>> {
        let server = MockServer::start();
        let formulae = server.mock(|when, then| {
            when.method(GET).path("/formula.json");
            then.status(200)
                .header("content-type", "application/json")
                .body(format!("[{JQ_FIXTURE}]"));
        });

        let repo = make_repo(&server);
        let result = repo.get_all_formulae().await?;

        formulae.assert();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "jq");
        Ok(())
    }

    #[tokio::test]
    async fn test_get_all_formulae_parses_mixed_uses_from_macos_shape()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = MockServer::start();
        let formulae = server.mock(|when, then| {
            when.method(GET).path("/formula.json");
            then.status(200)
                .header("content-type", "application/json")
                .body(format!("[{JQ_FIXTURE},{SEMGREP_FIXTURE}]"));
        });

        let repo = make_repo(&server);
        let result = repo.get_all_formulae().await?;

        formulae.assert();
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].name, "semgrep");
        assert_eq!(result[1].uses_from_macos.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_all_formulae_http_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/formula.json");
            then.status(503);
        });

        let repo = make_repo(&server);
        let result = repo.get_all_formulae().await;

        assert!(matches!(result, Err(FormulaError::Network(_))));
    }

    #[tokio::test]
    async fn test_get_ruby_source_success() -> Result<(), Box<dyn std::error::Error>> {
        let server = MockServer::start();
        let source = server.mock(|when, then| {
            when.method(GET).path("/Formula/t/test.rb");
            then.status(200)
                .header("content-type", "text/plain")
                .body("class Test < Formula\nend\n");
        });

        let repo = make_repo(&server);
        let result = repo.get_ruby_source("Formula/t/test.rb").await?;

        source.assert();
        assert!(result.contains("class Test < Formula"));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_ruby_source_rejects_encoded_traversal() {
        let server = MockServer::start();
        let repo = make_repo(&server);

        let result = repo.get_ruby_source("Formula/%2e%2e/secret.rb").await;

        assert!(matches!(
            result,
            Err(FormulaError::InvalidRubySourcePath { .. })
        ));
    }

    #[tokio::test]
    async fn test_get_ruby_source_does_not_escape_raw_source_prefix()
    -> Result<(), Box<dyn std::error::Error>> {
        let (base_url, captured_path) = spawn_capture_server()?;
        let repo = HttpFormulaRepository::with_urls(base_url.clone(), format!("{base_url}/root"));

        let result = repo.get_ruby_source("../secret.rb").await;
        let request_path = captured_path
            .lock()
            .map_err(|error| std::io::Error::other(error.to_string()))?
            .clone();

        assert_ne!(request_path.as_deref(), Some("/secret.rb"));
        if let Ok(body) = result {
            assert_eq!(body, "safe");
        }
        Ok(())
    }
}
