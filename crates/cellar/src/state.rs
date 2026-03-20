use std::path::Path;

use rusqlite::OptionalExtension;

use crate::error::CellarError;

/// SQLite-backed state database for tracking installed formulas.
pub struct StateDb {
    conn: rusqlite::Connection,
}

impl StateDb {
    /// Opens (or creates) the database at the given path.
    ///
    /// Creates parent directories and runs idempotent schema migration.
    ///
    /// # Errors
    ///
    /// Returns [`CellarError::Io`] if directory creation fails.
    /// Returns [`CellarError::Database`] if the database cannot be opened or migrated.
    pub fn open(path: &Path) -> Result<Self, CellarError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = rusqlite::Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS installs (
                name TEXT PRIMARY KEY,
                version TEXT NOT NULL,
                revision INTEGER NOT NULL DEFAULT 0,
                installed_on_request INTEGER NOT NULL DEFAULT 0,
                installed_at TEXT NOT NULL
            )",
        )?;
        Ok(Self { conn })
    }

    /// Inserts or replaces an install record.
    ///
    /// # Errors
    ///
    /// Returns [`CellarError::Database`] on query failure.
    pub fn insert(&self, record: &InstallRecord) -> Result<(), CellarError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO installs (name, version, revision, installed_on_request, installed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                record.name,
                record.version,
                record.revision,
                record.installed_on_request,
                record.installed_at,
            ],
        )?;
        Ok(())
    }

    /// Retrieves an install record by formula name.
    ///
    /// # Errors
    ///
    /// Returns [`CellarError::Database`] on query failure.
    pub fn get(&self, name: &str) -> Result<Option<InstallRecord>, CellarError> {
        let record = self
            .conn
            .query_row(
                "SELECT name, version, revision, installed_on_request, installed_at
                 FROM installs WHERE name = ?1",
                rusqlite::params![name],
                row_to_record,
            )
            .optional()?;
        Ok(record)
    }

    /// Lists all install records, ordered by name.
    ///
    /// # Errors
    ///
    /// Returns [`CellarError::Database`] on query failure.
    pub fn list(&self) -> Result<Vec<InstallRecord>, CellarError> {
        let mut stmt = self.conn.prepare(
            "SELECT name, version, revision, installed_on_request, installed_at
             FROM installs ORDER BY name",
        )?;
        let records = stmt
            .query_map([], row_to_record)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    /// Removes an install record by formula name.
    ///
    /// # Errors
    ///
    /// Returns [`CellarError::Database`] on query failure.
    pub fn remove(&self, name: &str) -> Result<(), CellarError> {
        self.conn.execute(
            "DELETE FROM installs WHERE name = ?1",
            rusqlite::params![name],
        )?;
        Ok(())
    }
}

/// Maps a `SQLite` row to an [`InstallRecord`].
fn row_to_record(row: &rusqlite::Row<'_>) -> Result<InstallRecord, rusqlite::Error> {
    Ok(InstallRecord {
        name: row.get(0)?,
        version: row.get(1)?,
        revision: row.get(2)?,
        installed_on_request: row.get(3)?,
        installed_at: row.get(4)?,
    })
}

/// A record of an installed formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRecord {
    /// Formula name.
    pub name: String,
    /// Installed version.
    pub version: String,
    /// Package revision.
    pub revision: u32,
    /// Whether the user explicitly requested this formula.
    pub installed_on_request: bool,
    /// Installation timestamp (ISO 8601).
    pub installed_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(name: &str) -> InstallRecord {
        InstallRecord {
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            revision: 0,
            installed_on_request: true,
            installed_at: "2024-01-15T12:00:00Z".to_owned(),
        }
    }

    #[test]
    fn test_state_db_insert_and_get() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let db = StateDb::open(&dir.path().join("test.db"))?;

        let record = sample_record("jq");
        db.insert(&record)?;

        let fetched = db.get("jq")?;
        assert_eq!(fetched, Some(record));
        Ok(())
    }

    #[test]
    fn test_state_db_get_missing_returns_none() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let db = StateDb::open(&dir.path().join("test.db"))?;

        assert_eq!(db.get("nonexistent")?, None);
        Ok(())
    }

    #[test]
    fn test_state_db_list() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let db = StateDb::open(&dir.path().join("test.db"))?;

        db.insert(&sample_record("zlib"))?;
        db.insert(&sample_record("jq"))?;
        db.insert(&sample_record("oniguruma"))?;

        let records = db.list()?;
        let names: Vec<_> = records.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["jq", "oniguruma", "zlib"]);
        Ok(())
    }

    #[test]
    fn test_state_db_remove() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let db = StateDb::open(&dir.path().join("test.db"))?;

        db.insert(&sample_record("jq"))?;
        assert!(db.get("jq")?.is_some());

        db.remove("jq")?;
        assert_eq!(db.get("jq")?, None);
        Ok(())
    }

    #[test]
    fn test_state_db_insert_replaces_existing() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let db = StateDb::open(&dir.path().join("test.db"))?;

        db.insert(&sample_record("jq"))?;

        let updated = InstallRecord {
            name: "jq".to_owned(),
            version: "2.0.0".to_owned(),
            revision: 1,
            installed_on_request: false,
            installed_at: "2024-06-01T00:00:00Z".to_owned(),
        };
        db.insert(&updated)?;

        let fetched = db.get("jq")?.ok_or("expected record")?;
        assert_eq!(fetched.version, "2.0.0");
        assert_eq!(fetched.revision, 1);
        assert!(!fetched.installed_on_request);
        Ok(())
    }

    #[test]
    fn test_state_db_idempotent_migration() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("test.db");

        let db = StateDb::open(&path)?;
        db.insert(&sample_record("jq"))?;
        drop(db);

        // Reopen — migration should not fail or lose data.
        let db = StateDb::open(&path)?;
        assert!(db.get("jq")?.is_some());
        Ok(())
    }

    #[test]
    fn test_state_db_remove_nonexistent_is_noop() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let db = StateDb::open(&dir.path().join("test.db"))?;

        db.remove("nonexistent")?;
        Ok(())
    }
}
