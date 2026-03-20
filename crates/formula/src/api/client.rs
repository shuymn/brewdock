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
#[derive(Debug)]
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
