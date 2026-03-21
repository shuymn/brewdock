use std::path::PathBuf;

use rusqlite::Connection;

use crate::{FormulaError, types::Formula};

/// In-memory representation of freshness metadata for the cached formula index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexMetadata {
    /// HTTP `ETag` returned by the upstream API, if any.
    pub etag: Option<String>,

    /// Unix timestamp (seconds) when the index was last fetched.
    pub fetched_at: u64,

    /// Number of formulae in the cached index.
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

/// `SQLite`-backed store for formula metadata with freshness tracking.
///
/// Stores a single `formula.db` file under the given cache directory with two
/// tables:
/// - `metadata` — singleton row for freshness tracking ([`IndexMetadata`])
/// - `formulae` — one row per formula, keyed by name with a JSON blob
///
/// This design supports efficient per-formula lookup without loading the entire
/// index, and enables future query patterns (search, listing) without a second
/// cache architecture.
pub struct MetadataStore {
    cache_dir: PathBuf,
}

impl MetadataStore {
    /// Creates a store rooted at the given cache directory.
    #[must_use]
    pub const fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Atomically persists the full formula index and freshness metadata.
    ///
    /// Replaces all existing formulae and metadata in a single transaction.
    /// Creates the cache directory and database if they do not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, the database cannot
    /// be opened, or the write fails.
    pub fn save_index(
        &self,
        formulae: &[Formula],
        metadata: &IndexMetadata,
    ) -> Result<(), FormulaError> {
        let mut conn = self.open_or_create()?;
        let tx = conn.transaction()?;

        tx.execute("DELETE FROM formulae", [])?;
        {
            let mut stmt = tx.prepare("INSERT INTO formulae (name, json_data) VALUES (?1, ?2)")?;
            for formula in formulae {
                let json = serde_json::to_string(formula)?;
                stmt.execute(rusqlite::params![formula.name, json])?;
            }
        }

        tx.execute(
            "INSERT OR REPLACE INTO metadata (id, etag, fetched_at, formula_count) \
             VALUES (1, ?1, ?2, ?3)",
            rusqlite::params![
                metadata.etag,
                i64::try_from(metadata.fetched_at).unwrap_or(i64::MAX),
                i64::try_from(metadata.formula_count).unwrap_or(i64::MAX)
            ],
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Loads freshness metadata from the database.
    ///
    /// Returns `Ok(None)` if the database does not exist or has no metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the database exists but cannot be read.
    pub fn load_metadata(&self) -> Result<Option<IndexMetadata>, FormulaError> {
        let Some(conn) = self.open_if_exists()? else {
            return Ok(None);
        };
        let mut stmt =
            conn.prepare("SELECT etag, fetched_at, formula_count FROM metadata WHERE id = 1")?;
        let result = stmt.query_row([], |row| {
            let etag: Option<String> = row.get(0)?;
            let fetched_at: i64 = row.get(1)?;
            let formula_count: i64 = row.get(2)?;
            Ok(IndexMetadata {
                etag,
                fetched_at: u64::try_from(fetched_at).unwrap_or(0),
                formula_count: usize::try_from(formula_count).unwrap_or(0),
            })
        });
        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(FormulaError::Database(err)),
        }
    }

    /// Looks up a single formula by name.
    ///
    /// Returns `Ok(None)` if the database does not exist or the formula is not
    /// cached.
    ///
    /// # Errors
    ///
    /// Returns an error if the database exists but the read or parse fails.
    pub fn load_formula(&self, name: &str) -> Result<Option<Formula>, FormulaError> {
        let Some(conn) = self.open_if_exists()? else {
            return Ok(None);
        };
        let mut stmt = conn.prepare("SELECT json_data FROM formulae WHERE name = ?1")?;
        let result = stmt.query_row(rusqlite::params![name], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        });
        match result {
            Ok(json) => {
                let formula: Formula = serde_json::from_str(&json)?;
                Ok(Some(formula))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(err) => Err(FormulaError::Database(err)),
        }
    }

    /// Returns the number of cached formulae without loading them.
    ///
    /// Returns `Ok(0)` if the database does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database exists but cannot be read.
    pub fn formula_count(&self) -> Result<usize, FormulaError> {
        let Some(conn) = self.open_if_exists()? else {
            return Ok(0);
        };
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM formulae", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Searches for formula names matching a SQL LIKE pattern.
    ///
    /// The `pattern` is matched against formula names using SQL `LIKE`
    /// (case-insensitive). Use `%` for wildcard matching.
    ///
    /// Returns an empty list if the database does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database exists but cannot be read.
    pub fn search_formulae(&self, pattern: &str) -> Result<Vec<String>, FormulaError> {
        let Some(conn) = self.open_if_exists()? else {
            return Ok(Vec::new());
        };
        let mut stmt =
            conn.prepare("SELECT name FROM formulae WHERE name LIKE ?1 ORDER BY name")?;
        let rows = stmt.query_map(rusqlite::params![pattern], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(FormulaError::Database)
    }

    /// Searches for formula names matching a SQL LIKE pattern with backslash
    /// escaping.
    ///
    /// The caller must pre-escape `%`, `_`, and `\` in the pattern.
    ///
    /// Returns an empty list if the database does not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the database exists but cannot be read.
    pub fn search_formulae_escaped(&self, pattern: &str) -> Result<Vec<String>, FormulaError> {
        let Some(conn) = self.open_if_exists()? else {
            return Ok(Vec::new());
        };
        let mut stmt =
            conn.prepare("SELECT name FROM formulae WHERE name LIKE ?1 ESCAPE '\\' ORDER BY name")?;
        let rows = stmt.query_map(rusqlite::params![pattern], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(FormulaError::Database)
    }

    /// Opens the database, creating it and the cache directory if needed.
    fn open_or_create(&self) -> Result<Connection, FormulaError> {
        std::fs::create_dir_all(&self.cache_dir)?;
        let conn = Connection::open(self.db_path())?;
        Self::apply_pragmas(&conn)?;
        Self::ensure_schema(&conn)?;
        Ok(conn)
    }

    /// Opens the database if it exists; returns `None` otherwise.
    ///
    /// Skips schema creation since the file already exists (schema was created
    /// by [`Self::open_or_create`] on first write).
    fn open_if_exists(&self) -> Result<Option<Connection>, FormulaError> {
        let path = self.db_path();
        if !path.exists() {
            return Ok(None);
        }
        let conn = Connection::open(&path)?;
        Self::apply_pragmas(&conn)?;
        Ok(Some(conn))
    }

    fn apply_pragmas(conn: &Connection) -> Result<(), FormulaError> {
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        Ok(())
    }

    fn ensure_schema(conn: &Connection) -> Result<(), FormulaError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS metadata (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                etag TEXT,
                fetched_at INTEGER NOT NULL,
                formula_count INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS formulae (
                name TEXT PRIMARY KEY NOT NULL,
                json_data TEXT NOT NULL
            );",
        )?;
        Ok(())
    }

    fn db_path(&self) -> PathBuf {
        self.cache_dir.join("formula.db")
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

    fn sample_metadata() -> IndexMetadata {
        IndexMetadata {
            etag: Some("\"abc123\"".to_owned()),
            fetched_at: 1_711_036_800,
            formula_count: 2,
        }
    }

    #[test]
    fn test_load_metadata_returns_none_when_no_db() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        assert!(store.load_metadata()?.is_none());
        Ok(())
    }

    #[test]
    fn test_save_index_and_load_metadata() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        let formulae = vec![test_formula("jq", &["oniguruma"])];
        let meta = sample_metadata();

        store.save_index(&formulae, &meta)?;
        let loaded = store.load_metadata()?;

        assert_eq!(loaded, Some(meta));
        Ok(())
    }

    #[test]
    fn test_save_index_metadata_without_etag() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        let meta = IndexMetadata {
            etag: None,
            fetched_at: 1_711_036_800,
            formula_count: 0,
        };

        store.save_index(&[], &meta)?;
        let loaded = store.load_metadata()?;

        assert_eq!(loaded, Some(meta));
        Ok(())
    }

    #[test]
    fn test_load_formula_returns_none_when_no_db() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        assert!(store.load_formula("jq")?.is_none());
        Ok(())
    }

    #[test]
    fn test_load_formula_returns_none_when_not_cached() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        store.save_index(&[test_formula("wget", &[])], &sample_metadata())?;
        assert!(store.load_formula("jq")?.is_none());
        Ok(())
    }

    #[test]
    fn test_load_formula_returns_cached_formula() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        let formulae = vec![
            test_formula("jq", &["oniguruma"]),
            test_formula("wget", &[]),
        ];
        store.save_index(&formulae, &sample_metadata())?;

        let formula = store
            .load_formula("jq")?
            .ok_or("expected jq to be cached")?;
        assert_eq!(formula.name, "jq");
        assert_eq!(formula.dependencies, vec!["oniguruma"]);
        Ok(())
    }

    #[test]
    fn test_save_index_creates_cache_directory() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let nested = dir.path().join("nested").join("cache");
        let store = MetadataStore::new(nested.clone());

        store.save_index(
            &[],
            &IndexMetadata {
                etag: None,
                fetched_at: 0,
                formula_count: 0,
            },
        )?;

        assert!(nested.exists());
        Ok(())
    }

    #[test]
    fn test_save_index_replaces_previous_data() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;

        store.save_index(
            &[test_formula("jq", &[]), test_formula("wget", &[])],
            &IndexMetadata {
                etag: Some("\"v1\"".to_owned()),
                fetched_at: 100,
                formula_count: 2,
            },
        )?;

        store.save_index(
            &[test_formula("curl", &[])],
            &IndexMetadata {
                etag: Some("\"v2\"".to_owned()),
                fetched_at: 200,
                formula_count: 1,
            },
        )?;

        assert!(store.load_formula("jq")?.is_none());
        assert!(store.load_formula("wget")?.is_none());
        assert!(store.load_formula("curl")?.is_some());

        let meta = store.load_metadata()?.ok_or("metadata should exist")?;
        assert_eq!(meta.etag.as_deref(), Some("\"v2\""));
        assert_eq!(meta.formula_count, 1);
        Ok(())
    }

    #[test]
    fn test_formula_count() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        assert_eq!(store.formula_count()?, 0);

        store.save_index(
            &[test_formula("jq", &[]), test_formula("wget", &[])],
            &sample_metadata(),
        )?;
        assert_eq!(store.formula_count()?, 2);
        Ok(())
    }

    #[test]
    fn test_search_formulae() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        store.save_index(
            &[
                test_formula("jq", &[]),
                test_formula("jql", &[]),
                test_formula("wget", &[]),
            ],
            &IndexMetadata {
                etag: None,
                fetched_at: 0,
                formula_count: 3,
            },
        )?;

        let results = store.search_formulae("jq%")?;
        assert_eq!(results, vec!["jq", "jql"]);

        let results = store.search_formulae("%get")?;
        assert_eq!(results, vec!["wget"]);

        let results = store.search_formulae("nonexistent%")?;
        assert!(results.is_empty());
        Ok(())
    }

    #[test]
    fn test_search_formulae_returns_empty_when_no_db() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, store) = temp_store()?;
        let results = store.search_formulae("jq%")?;
        assert!(results.is_empty());
        Ok(())
    }
}
