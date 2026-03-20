use crate::{error::FormulaError, types::Formula};

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
}

/// HTTP-based formula repository using the Homebrew JSON API.
#[derive(Debug, Clone)]
pub struct HttpFormulaRepository {
    client: reqwest::Client,
    base_url: String,
}

impl HttpFormulaRepository {
    /// Creates a repository pointing at the default Homebrew API.
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: "https://formulae.brew.sh/api".to_owned(),
        }
    }

    /// Creates a repository with a custom base URL.
    #[must_use]
    pub fn with_base_url(base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
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
}

#[cfg(test)]
mod tests {
    use httpmock::{Method::GET, MockServer};

    use super::*;

    const JQ_FIXTURE: &str = include_str!("../../tests/fixtures/formula/jq.json");

    fn make_repo(server: &MockServer) -> HttpFormulaRepository {
        HttpFormulaRepository::with_base_url(server.base_url())
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
}
