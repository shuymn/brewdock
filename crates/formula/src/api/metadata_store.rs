use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{FormulaError, types::Formula};

/// On-disk metadata associated with the cached formula index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexMetadata {
    /// HTTP `ETag` returned by the upstream API, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,

    /// Unix timestamp (seconds) when the index was last fetched.
    pub fetched_at: u64,

    /// Number of formulae in the cached index.
    #[serde(default)]
    pub formula_count: usize,
}

/// Result of a conditional formula index fetch.
#[derive(Debug)]
pub enum FetchOutcome {
    /// The upstream data changed; contains the new formulae and optional `ETag`.
    Modified {
        /// The updated formula list.
        formulae: Vec<Formula>,
        /// `ETag` from the HTTP response, if present.
        etag: Option<String>,
    },
    /// The upstream data has not changed since the provided `ETag` (HTTP 304).
    NotModified,
}

/// Manages on-disk persistence of formula metadata with freshness tracking.
///
/// Stores two files under the given cache directory:
/// - `formula.json` — the full formula index (`Vec<Formula>`)
/// - `formula-meta.json` — freshness metadata ([`IndexMetadata`])
///
/// The split avoids parsing the large formula list when only freshness
/// information is needed (e.g., reading the `ETag` before a conditional fetch).
pub struct MetadataStore {
    cache_dir: PathBuf,
}

impl MetadataStore {
    /// Creates a store rooted at the given cache directory.
    #[must_use]
    pub const fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Loads freshness metadata from disk.
    ///
    /// Returns `Ok(None)` if the metadata file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load_metadata(&self) -> Result<Option<IndexMetadata>, FormulaError> {
        load_json(&self.meta_path())
    }

    /// Saves freshness metadata to disk.
    ///
    /// Creates the cache directory if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the file
    /// cannot be written.
    pub fn save_metadata(&self, meta: &IndexMetadata) -> Result<(), FormulaError> {
        self.save_json(&self.meta_path(), meta)
    }

    /// Loads the cached formula list from disk.
    ///
    /// Returns `Ok(None)` if the formula file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load_formulae(&self) -> Result<Option<Vec<Formula>>, FormulaError> {
        load_json(&self.formulae_path())
    }

    /// Saves the formula list to disk.
    ///
    /// Creates the cache directory if it does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the file
    /// cannot be written.
    pub fn save_formulae(&self, formulae: &[Formula]) -> Result<(), FormulaError> {
        self.save_json(&self.formulae_path(), formulae)
    }

    /// Checks whether the formula index file exists on disk without parsing it.
    #[must_use]
    pub fn has_formulae(&self) -> bool {
        self.formulae_path().exists()
    }

    /// Loads the cached formulae as a map keyed by formula name.
    ///
    /// Returns `Ok(None)` if the formula file does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load_formula_map(&self) -> Result<Option<HashMap<String, Formula>>, FormulaError> {
        self.load_formulae().map(|opt| {
            opt.map(|formulae| formulae.into_iter().map(|f| (f.name.clone(), f)).collect())
        })
    }

    fn save_json(
        &self,
        path: &Path,
        value: &(impl Serialize + ?Sized),
    ) -> Result<(), FormulaError> {
        std::fs::create_dir_all(&self.cache_dir).map_err(FormulaError::Io)?;
        let data = serde_json::to_vec(value)?;
        std::fs::write(path, &data).map_err(FormulaError::Io)?;
        Ok(())
    }

    fn formulae_path(&self) -> PathBuf {
        self.cache_dir.join("formula.json")
    }

    fn meta_path(&self) -> PathBuf {
        self.cache_dir.join("formula-meta.json")
    }
}

fn load_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, FormulaError> {
    match std::fs::read(path) {
        Ok(data) => {
            let value: T = serde_json::from_slice(&data)?;
            Ok(Some(value))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(FormulaError::Io(err)),
    }
}

impl std::fmt::Debug for MetadataStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetadataStore")
            .field("cache_dir", &self.cache_dir)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_formula;

    fn temp_store() -> Result<(tempfile::TempDir, MetadataStore), std::io::Error> {
        let dir = tempfile::tempdir()?;
        let store = MetadataStore::new(dir.path().join("cache"));
        Ok((dir, store))
    }

    #[test]
    fn test_load_metadata_returns_none_when_no_file() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        assert!(store.load_metadata()?.is_none());
        Ok(())
    }

    #[test]
    fn test_save_and_load_metadata() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        let meta = IndexMetadata {
            etag: Some("\"abc123\"".to_owned()),
            fetched_at: 1_711_036_800,
            formula_count: 42,
        };

        store.save_metadata(&meta)?;
        let loaded = store.load_metadata()?;

        assert_eq!(loaded, Some(meta));
        Ok(())
    }

    #[test]
    fn test_save_metadata_without_etag() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        let meta = IndexMetadata {
            etag: None,
            fetched_at: 1_711_036_800,
            formula_count: 0,
        };

        store.save_metadata(&meta)?;
        let loaded = store.load_metadata()?;

        assert_eq!(loaded, Some(meta));
        Ok(())
    }

    #[test]
    fn test_load_formulae_returns_none_when_no_file() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        assert!(store.load_formulae()?.is_none());
        Ok(())
    }

    #[test]
    fn test_save_and_load_formulae() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        let formulae = vec![
            test_formula("jq", &["oniguruma"]),
            test_formula("wget", &[]),
        ];

        store.save_formulae(&formulae)?;
        let loaded = store
            .load_formulae()?
            .ok_or("formulae should exist after save")?;

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, formulae[0].name);
        assert_eq!(loaded[1].name, formulae[1].name);
        Ok(())
    }

    #[test]
    fn test_load_formula_map() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        let formulae = vec![
            test_formula("jq", &["oniguruma"]),
            test_formula("wget", &[]),
        ];

        store.save_formulae(&formulae)?;
        let map = store
            .load_formula_map()?
            .ok_or("formula map should exist after save")?;

        assert_eq!(map.len(), 2);
        assert!(map.contains_key("jq"));
        assert!(map.contains_key("wget"));
        assert_eq!(map["jq"].dependencies, vec!["oniguruma"]);
        Ok(())
    }

    #[test]
    fn test_load_formula_map_returns_none_when_no_file() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        assert!(store.load_formula_map()?.is_none());
        Ok(())
    }

    #[test]
    fn test_save_creates_cache_directory() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let nested = dir.path().join("nested").join("cache");
        let store = MetadataStore::new(nested.clone());

        store.save_metadata(&IndexMetadata {
            etag: None,
            fetched_at: 0,
            formula_count: 0,
        })?;

        assert!(nested.exists());
        Ok(())
    }
}
