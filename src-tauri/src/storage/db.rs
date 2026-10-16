//! SQLite schema and access.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

/// The single database handle. Wrapped in a mutex because `rusqlite::Connection` is not `Sync`.
pub struct Database {
    connection: Mutex<Connection>,
    path: PathBuf,
}

impl Database {
    /// Opens (creating if needed) the database at `path` and applies migrations.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let connection = Connection::open(path).map_err(|error| error.to_string())?;

        // WAL keeps readers from blocking the writer and survives power loss better.
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| error.to_string())?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| error.to_string())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| error.to_string())?;

        let database = Database {
            connection: Mutex::new(connection),
            path: path.to_path_buf(),
        };
        database.migrate()?;
        Ok(database)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn migrate(&self) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "The database lock was poisoned.".to_string())?;

        // Do not drop legacy tables or reset user_version: preserve existing user data.
        connection.execute_batch(r#"
                    CREATE TABLE IF NOT EXISTS settings (
                        id INTEGER PRIMARY KEY CHECK (id = 1),
                        payload TEXT NOT NULL
                    );

                    CREATE TABLE IF NOT EXISTS scan_history (
                        scan_id TEXT PRIMARY KEY,
                        root_path TEXT NOT NULL,
                        root_name TEXT NOT NULL,
                        started_at INTEGER NOT NULL,
                        finished_at INTEGER,
                        duration_ms INTEGER,
                        status TEXT NOT NULL,
                        total_bytes INTEGER NOT NULL,
                        file_count INTEGER NOT NULL,
                        folder_count INTEGER NOT NULL,
                        error_count INTEGER NOT NULL,
                        truncated INTEGER NOT NULL,
                        drive_id TEXT,
                        persisted INTEGER NOT NULL DEFAULT 0
                    );

                    CREATE INDEX IF NOT EXISTS scan_history_started_at
                        ON scan_history (started_at DESC);

                    CREATE TABLE IF NOT EXISTS pro_entitlement (
                        id INTEGER PRIMARY KEY CHECK (id = 1),
                        payload TEXT NOT NULL
                    );

        "#).map_err(|error| error.to_string())?;
        Ok(())
    }

    /* ------------------------------- settings ------------------------------- */

    pub fn load_settings(&self) -> Result<Option<String>, String> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT payload FROM settings WHERE id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn save_settings(&self, payload: &str) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO settings (id, payload) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET payload = excluded.payload",
                params![payload],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /* ----------------------------- scan history ----------------------------- */

    pub fn insert_scan_history(&self, row: &ScanHistoryRow) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO scan_history (
                    scan_id, root_path, root_name, started_at, finished_at, duration_ms,
                    status, total_bytes, file_count, folder_count, error_count, truncated,
                    drive_id, persisted
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(scan_id) DO UPDATE SET
                    finished_at = excluded.finished_at,
                    duration_ms = excluded.duration_ms,
                    status = excluded.status,
                    total_bytes = excluded.total_bytes,
                    file_count = excluded.file_count,
                    folder_count = excluded.folder_count,
                    error_count = excluded.error_count,
                    truncated = excluded.truncated",
                params![
                    row.scan_id,
                    row.root_path,
                    row.root_name,
                    row.started_at,
                    row.finished_at,
                    row.duration_ms,
                    row.status,
                    row.total_bytes,
                    row.file_count,
                    row.folder_count,
                    row.error_count,
                    row.truncated as i64,
                    row.drive_id,
                    row.persisted as i64,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn list_scan_history(&self, limit: usize) -> Result<Vec<ScanHistoryRow>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT scan_id, root_path, root_name, started_at, finished_at, duration_ms,
                        status, total_bytes, file_count, folder_count, error_count, truncated,
                        drive_id, persisted
                 FROM scan_history ORDER BY started_at DESC LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map(params![limit as i64], |row| {
                Ok(ScanHistoryRow {
                    scan_id: row.get(0)?,
                    root_path: row.get(1)?,
                    root_name: row.get(2)?,
                    started_at: row.get(3)?,
                    finished_at: row.get(4)?,
                    duration_ms: row.get(5)?,
                    status: row.get(6)?,
                    total_bytes: row.get(7)?,
                    file_count: row.get(8)?,
                    folder_count: row.get(9)?,
                    error_count: row.get(10)?,
                    truncated: row.get::<_, i64>(11)? != 0,
                    drive_id: row.get(12)?,
                    persisted: row.get::<_, i64>(13)? != 0,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;

        Ok(rows)
    }

    /* ---------------------------- entitlement ---------------------------- */

    /// The persisted Diskora Pro entitlement, if this device has a
    /// verified purchase. The payload is a JSON `StoredEntitlement`.
    pub fn load_entitlement(&self) -> Result<Option<String>, String> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT payload FROM pro_entitlement WHERE id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn save_entitlement(&self, payload: &str) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO pro_entitlement (id, payload) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET payload = excluded.payload",
                params![payload],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn clear_entitlement(&self) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute("DELETE FROM pro_entitlement WHERE id = 1", [])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "The database lock was poisoned.".to_string())
    }
}

/// One row of `scan_history`. Mirrors the fields of `ScanSummary` the UI needs.
#[derive(Debug, Clone)]
pub struct ScanHistoryRow {
    pub scan_id: String,
    pub root_path: String,
    pub root_name: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub status: String,
    pub total_bytes: i64,
    pub file_count: i64,
    pub folder_count: i64,
    pub error_count: i64,
    pub truncated: bool,
    pub drive_id: Option<String>,
    pub persisted: bool,
}

/// One row of `device_activations`.
#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("diskwise-db-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base.join("diskwise.sqlite3")
    }

    #[test]
    fn settings_round_trip() {
        let path = temp_db("settings");
        let database = Database::open(&path).unwrap();
        assert!(database.load_settings().unwrap().is_none());
        database.save_settings("{\"theme\":\"dark\"}").unwrap();
        assert_eq!(
            database.load_settings().unwrap().as_deref(),
            Some("{\"theme\":\"dark\"}")
        );
        // Overwrite.
        database.save_settings("{\"theme\":\"light\"}").unwrap();
        assert_eq!(
            database.load_settings().unwrap().as_deref(),
            Some("{\"theme\":\"light\"}")
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn scan_history_round_trips_and_orders_by_recency() {
        let path = temp_db("history");
        let database = Database::open(&path).unwrap();
        for index in 0..3 {
            database
                .insert_scan_history(&ScanHistoryRow {
                    scan_id: format!("scan-{index}"),
                    root_path: "C:\\".to_string(),
                    root_name: "Local Disk (C:)".to_string(),
                    started_at: 1000 + index as i64,
                    finished_at: Some(2000 + index as i64),
                    duration_ms: Some(1000),
                    status: "complete".to_string(),
                    total_bytes: 1024,
                    file_count: 10,
                    folder_count: 2,
                    error_count: 0,
                    truncated: false,
                    drive_id: Some("C:".to_string()),
                    persisted: true,
                })
                .unwrap();
        }
        let rows = database.list_scan_history(10).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].scan_id, "scan-2", "newest first");
        assert!(rows[0].persisted);
        assert_eq!(rows[0].total_bytes, 1024);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn migrations_are_idempotent() {
        let path = temp_db("migrate");
        {
            let database = Database::open(&path).unwrap();
            drop(database);
        }
        let database = Database::open(&path).unwrap();
        database.save_settings("{}").unwrap();
        assert!(database.load_settings().unwrap().is_some());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn entitlement_round_trips_and_clears() {
        let path = temp_db("entitlement");
        let database = Database::open(&path).unwrap();
        assert!(database.load_entitlement().unwrap().is_none());
        database.save_entitlement("{\"licenseKey\":\"ABCD\"}").unwrap();
        assert_eq!(
            database.load_entitlement().unwrap().as_deref(),
            Some("{\"licenseKey\":\"ABCD\"}")
        );
        database.clear_entitlement().unwrap();
        assert!(database.load_entitlement().unwrap().is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
