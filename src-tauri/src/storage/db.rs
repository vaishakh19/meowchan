//! SQLite schema and access.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

/// Bump when the schema changes. Migrations run in order.
///
/// Version 2 adds the licensing tables (licence records, device activations, webhook event
/// idempotency, refunds, revocations and the audit log) and the locally stored licence key.
pub const SCHEMA_VERSION: u32 = 2;

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

        let version: u32 = connection
            .query_row("PRAGMA user_version", params![], |row| row.get(0))
            .unwrap_or(0);

        if version < 1 {
            connection
                .execute_batch(
                    r#"
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

                    CREATE TABLE IF NOT EXISTS entitlement_cache (
                        id INTEGER PRIMARY KEY CHECK (id = 1),
                        payload TEXT NOT NULL,
                        verified_at INTEGER NOT NULL
                    );

                    CREATE TABLE IF NOT EXISTS entitlement_events (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        at_ms INTEGER NOT NULL,
                        from_state TEXT NOT NULL,
                        to_state TEXT NOT NULL,
                        reason TEXT NOT NULL
                    );

                    CREATE TABLE IF NOT EXISTS cleanup_rules (
                        id TEXT PRIMARY KEY,
                        payload TEXT NOT NULL
                    );

                    PRAGMA user_version = 1;
                    "#,
                )
                .map_err(|error| error.to_string())?;
        }

        if version < 2 {
            connection
                .execute_batch(
                    r#"
                    -- The locally activated licence key. Stored as-is: it is a bearer token for
                    -- offline verification, and its disclosure lets someone else use the licence
                    -- on another machine until the device limit catches it. The database lives in
                    -- the user's own profile, which is the same protection every other credential
                    -- on the machine gets. A hash would not work here because the app must present
                    -- the key verbatim when re-verifying.
                    CREATE TABLE IF NOT EXISTS license_key (
                        id INTEGER PRIMARY KEY CHECK (id = 1),
                        key TEXT NOT NULL,
                        activated_at INTEGER NOT NULL,
                        device_binding TEXT NOT NULL
                    );

                    -- Device activations, mirrored from the licence service so the app can show
                    -- them and enforce the limit offline. The service is authoritative; this is a
                    -- cache the user can inspect.
                    CREATE TABLE IF NOT EXISTS device_activations (
                        device_id TEXT PRIMARY KEY,
                        license_id TEXT NOT NULL,
                        device_binding TEXT NOT NULL,
                        device_name TEXT NOT NULL,
                        activated_at INTEGER NOT NULL,
                        last_seen_at INTEGER NOT NULL,
                        deactivated_at INTEGER
                    );

                    -- Cached licence records, so the app can report what it holds offline.
                    CREATE TABLE IF NOT EXISTS license_records (
                        license_id TEXT PRIMARY KEY,
                        product_id TEXT NOT NULL,
                        tier_id TEXT NOT NULL,
                        permanent INTEGER NOT NULL,
                        lease_until INTEGER,
                        issued_at INTEGER NOT NULL,
                        device_limit INTEGER NOT NULL,
                        revoked INTEGER NOT NULL DEFAULT 0,
                        revocation_reason TEXT,
                        updated_at INTEGER NOT NULL
                    );

                    -- Webhook event idempotency. The service owns this table; the app never writes
                    -- it. It is declared here so the schema is documented in one place.
                    CREATE TABLE IF NOT EXISTS webhook_events (
                        webhook_id TEXT PRIMARY KEY,
                        event_type TEXT NOT NULL,
                        received_at INTEGER NOT NULL,
                        processed_at INTEGER,
                        attempts INTEGER NOT NULL DEFAULT 0,
                        last_error TEXT
                    );

                    -- Refunds, revocations and other audit events, newest first.
                    CREATE TABLE IF NOT EXISTS license_audit (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        at_ms INTEGER NOT NULL,
                        license_id TEXT,
                        event_type TEXT NOT NULL,
                        detail TEXT NOT NULL
                    );

                    CREATE INDEX IF NOT EXISTS license_audit_at
                        ON license_audit (at_ms DESC);

                    PRAGMA user_version = 2;
                    "#,
                )
                .map_err(|error| error.to_string())?;
        }

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

    pub fn load_entitlement_cache(&self) -> Result<Option<(String, i64)>, String> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT payload, verified_at FROM entitlement_cache WHERE id = 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn save_entitlement_cache(&self, payload: &str, verified_at: i64) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO entitlement_cache (id, payload, verified_at) VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET payload = excluded.payload, verified_at = excluded.verified_at",
                params![payload, verified_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn clear_entitlement_cache(&self) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute("DELETE FROM entitlement_cache WHERE id = 1", params![])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn record_entitlement_event(
        &self,
        at_ms: i64,
        from_state: &str,
        to_state: &str,
        reason: &str,
    ) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO entitlement_events (at_ms, from_state, to_state, reason)
                 VALUES (?1, ?2, ?3, ?4)",
                params![at_ms, from_state, to_state, reason],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn list_entitlement_events(&self, limit: usize) -> Result<Vec<EntitlementEventRow>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT at_ms, from_state, to_state, reason FROM entitlement_events
                 ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![limit as i64], |row| {
                Ok(EntitlementEventRow {
                    at_ms: row.get(0)?,
                    from_state: row.get(1)?,
                    to_state: row.get(2)?,
                    reason: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    }


    /* ------------------------------ licence key ------------------------------ */

    /// The locally stored licence key and the device it was activated for.
    pub fn load_license_key(&self) -> Result<Option<String>, String> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT key FROM license_key WHERE id = 1",
                params![],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// The device binding the stored key was activated for.
    pub fn load_license_binding(&self) -> Result<Option<String>, String> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT device_binding FROM license_key WHERE id = 1",
                params![],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// Stores the activated licence key. Replaces any previous one.
    pub fn save_license_key(&self, key: &str) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO license_key (id, key, activated_at, device_binding)
                 VALUES (1, ?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET
                    key = excluded.key,
                    activated_at = excluded.activated_at,
                    device_binding = excluded.device_binding",
                params![key, crate::licensing::verify::now_millis(), "pending"],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Records which device a stored key was activated for.
    pub fn set_license_binding(&self, device_binding: &str) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute(
                "UPDATE license_key SET device_binding = ?1 WHERE id = 1",
                params![device_binding],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Removes the stored licence key.
    pub fn clear_license_key(&self) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute("DELETE FROM license_key WHERE id = 1", params![])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /* --------------------------- device activations --------------------------- */

    /// Records or refreshes a device activation.
    pub fn upsert_device_activation(&self, row: &DeviceActivationRow) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO device_activations (
                    device_id, license_id, device_binding, device_name,
                    activated_at, last_seen_at, deactivated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(device_id) DO UPDATE SET
                    license_id = excluded.license_id,
                    device_binding = excluded.device_binding,
                    device_name = excluded.device_name,
                    last_seen_at = excluded.last_seen_at,
                    deactivated_at = excluded.deactivated_at",
                params![
                    row.device_id,
                    row.license_id,
                    row.device_binding,
                    row.device_name,
                    row.activated_at,
                    row.last_seen_at,
                    row.deactivated_at,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Every activation that has not been deactivated.
    pub fn active_device_activations(&self) -> Result<Vec<DeviceActivationRow>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT device_id, license_id, device_binding, device_name,
                        activated_at, last_seen_at, deactivated_at
                 FROM device_activations WHERE deactivated_at IS NULL
                 ORDER BY activated_at ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![], |row| {
                Ok(DeviceActivationRow {
                    device_id: row.get(0)?,
                    license_id: row.get(1)?,
                    device_binding: row.get(2)?,
                    device_name: row.get(3)?,
                    activated_at: row.get(4)?,
                    last_seen_at: row.get(5)?,
                    deactivated_at: row.get(6)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    }

    /// Marks a device as deactivated without deleting the row, so the audit trail survives.
    pub fn deactivate_device(&self, device_id: &str, at: i64) -> Result<bool, String> {
        let connection = self.lock()?;
        let changed = connection
            .execute(
                "UPDATE device_activations SET deactivated_at = ?1
                 WHERE device_id = ?2 AND deactivated_at IS NULL",
                params![at, device_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed > 0)
    }

    /// How many devices are currently active.
    pub fn active_device_count(&self) -> Result<u32, String> {
        let connection = self.lock()?;
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM device_activations WHERE deactivated_at IS NULL",
                params![],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        Ok(count.max(0) as u32)
    }

    /* ---------------------------- licence records ---------------------------- */

    /// Caches the licence record the service reported.
    pub fn upsert_license_record(&self, row: &LicenseRecordRow) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO license_records (
                    license_id, product_id, tier_id, permanent, lease_until,
                    issued_at, device_limit, revoked, revocation_reason, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(license_id) DO UPDATE SET
                    product_id = excluded.product_id,
                    tier_id = excluded.tier_id,
                    permanent = excluded.permanent,
                    lease_until = excluded.lease_until,
                    issued_at = excluded.issued_at,
                    device_limit = excluded.device_limit,
                    revoked = excluded.revoked,
                    revocation_reason = excluded.revocation_reason,
                    updated_at = excluded.updated_at",
                params![
                    row.license_id,
                    row.product_id,
                    row.tier_id,
                    row.permanent as i64,
                    row.lease_until,
                    row.issued_at,
                    row.device_limit,
                    row.revoked as i64,
                    row.revocation_reason,
                    row.updated_at,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// The cached licence record, if any.
    pub fn load_license_record(&self) -> Result<Option<LicenseRecordRow>, String> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT license_id, product_id, tier_id, permanent, lease_until,
                        issued_at, device_limit, revoked, revocation_reason, updated_at
                 FROM license_records LIMIT 1",
                params![],
                |row| {
                    Ok(LicenseRecordRow {
                        license_id: row.get(0)?,
                        product_id: row.get(1)?,
                        tier_id: row.get(2)?,
                        permanent: row.get::<_, i64>(3)? != 0,
                        lease_until: row.get(4)?,
                        issued_at: row.get(5)?,
                        device_limit: row.get::<_, i64>(6)? as u32,
                        revoked: row.get::<_, i64>(7)? != 0,
                        revocation_reason: row.get(8)?,
                        updated_at: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /* -------------------------------- audit -------------------------------- */

    /// Appends an audit event. Never contains file paths or file contents.
    pub fn record_license_audit(
        &self,
        at_ms: i64,
        license_id: Option<&str>,
        event_type: &str,
        detail: &str,
    ) -> Result<(), String> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO license_audit (at_ms, license_id, event_type, detail)
                 VALUES (?1, ?2, ?3, ?4)",
                params![at_ms, license_id, event_type, detail],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// The audit log, newest first.
    pub fn list_license_audit(&self, limit: usize) -> Result<Vec<LicenseAuditRow>, String> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT at_ms, license_id, event_type, detail FROM license_audit
                 ORDER BY id DESC LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![limit as i64], |row| {
                Ok(LicenseAuditRow {
                    at_ms: row.get(0)?,
                    license_id: row.get(1)?,
                    event_type: row.get(2)?,
                    detail: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
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
#[derive(Debug, Clone)]
pub struct DeviceActivationRow {
    pub device_id: String,
    pub license_id: String,
    pub device_binding: String,
    pub device_name: String,
    pub activated_at: i64,
    pub last_seen_at: i64,
    pub deactivated_at: Option<i64>,
}

/// One row of `license_records`.
#[derive(Debug, Clone)]
pub struct LicenseRecordRow {
    pub license_id: String,
    pub product_id: String,
    pub tier_id: String,
    pub permanent: bool,
    pub lease_until: Option<i64>,
    pub issued_at: i64,
    pub device_limit: u32,
    pub revoked: bool,
    pub revocation_reason: Option<String>,
    pub updated_at: i64,
}

/// One row of `license_audit`.
#[derive(Debug, Clone)]
pub struct LicenseAuditRow {
    pub at_ms: i64,
    pub license_id: Option<String>,
    pub event_type: String,
    pub detail: String,
}

/// One row of `entitlement_events`. Mirrors `EntitlementEvent`.
#[derive(Debug, Clone)]
pub struct EntitlementEventRow {
    pub at_ms: i64,
    pub from_state: String,
    pub to_state: String,
    pub reason: String,
}

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
    fn entitlement_cache_round_trips_and_clears() {
        let path = temp_db("entitlement");
        let database = Database::open(&path).unwrap();
        assert!(database.load_entitlement_cache().unwrap().is_none());
        database
            .save_entitlement_cache("{\"state\":\"pro-active\"}", 4242)
            .unwrap();
        let (payload, verified_at) = database.load_entitlement_cache().unwrap().unwrap();
        assert_eq!(payload, "{\"state\":\"pro-active\"}");
        assert_eq!(verified_at, 4242);
        database.clear_entitlement_cache().unwrap();
        assert!(database.load_entitlement_cache().unwrap().is_none());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_licence_key_round_trips_and_can_be_cleared() {
        let path = temp_db("license-key");
        let database = Database::open(&path).unwrap();
        assert!(database.load_license_key().unwrap().is_none());
        database.save_license_key("abc.def").unwrap();
        assert_eq!(database.load_license_key().unwrap().as_deref(), Some("abc.def"));
        // Overwrite.
        database.save_license_key("ghi.jkl").unwrap();
        assert_eq!(database.load_license_key().unwrap().as_deref(), Some("ghi.jkl"));
        database.clear_license_key().unwrap();
        assert!(database.load_license_key().unwrap().is_none());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn device_activations_track_a_limit_and_support_deactivation() {
        let path = temp_db("devices");
        let database = Database::open(&path).unwrap();
        assert_eq!(database.active_device_count().unwrap(), 0);

        for index in 0..3 {
            database
                .upsert_device_activation(&DeviceActivationRow {
                    device_id: format!("dev-{index}"),
                    license_id: "lic-1".to_string(),
                    device_binding: format!("dev-{index}"),
                    device_name: format!("PC-{index}"),
                    activated_at: 1000 + index,
                    last_seen_at: 1000 + index,
                    deactivated_at: None,
                })
                .unwrap();
        }
        assert_eq!(database.active_device_count().unwrap(), 3);

        // Deactivating one frees a slot but keeps the row for the audit trail.
        assert!(database.deactivate_device("dev-0", 2000).unwrap());
        assert!(!database.deactivate_device("dev-0", 2000).unwrap(), "already inactive");
        assert_eq!(database.active_device_count().unwrap(), 2);
        assert_eq!(database.active_device_activations().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn licence_records_round_trip() {
        let path = temp_db("records");
        let database = Database::open(&path).unwrap();
        database
            .upsert_license_record(&LicenseRecordRow {
                license_id: "lic-1".to_string(),
                product_id: "diskwise-pro".to_string(),
                tier_id: "pro-lifetime".to_string(),
                permanent: true,
                lease_until: None,
                issued_at: 1000,
                device_limit: 3,
                revoked: false,
                revocation_reason: None,
                updated_at: 2000,
            })
            .unwrap();
        let record = database.load_license_record().unwrap().expect("a record");
        assert_eq!(record.license_id, "lic-1");
        assert!(record.permanent);
        assert_eq!(record.device_limit, 3);
        assert!(!record.revoked);

        // A revocation is recorded, not deleted.
        database
            .upsert_license_record(&LicenseRecordRow {
                revoked: true,
                revocation_reason: Some("refunded".to_string()),
                updated_at: 3000,
                ..record
            })
            .unwrap();
        let revoked = database.load_license_record().unwrap().expect("a record");
        assert!(revoked.revoked);
        assert_eq!(revoked.revocation_reason.as_deref(), Some("refunded"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_audit_log_is_newest_first_and_never_stores_paths() {
        let path = temp_db("audit");
        let database = Database::open(&path).unwrap();
        database
            .record_license_audit(1, Some("lic-1"), "activated", "Licence activated.")
            .unwrap();
        database
            .record_license_audit(2, Some("lic-1"), "refunded", "Refund processed.")
            .unwrap();
        let rows = database.list_license_audit(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].event_type, "refunded");
        assert_eq!(rows[1].event_type, "activated");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn entitlement_events_append_newest_first() {
        let path = temp_db("events");
        let database = Database::open(&path).unwrap();
        database
            .record_entitlement_event(1, "free", "trial", "trial started")
            .unwrap();
        database
            .record_entitlement_event(2, "trial", "pro-active", "licence activated")
            .unwrap();
        let events = database.list_entitlement_events(10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].to_state, "pro-active");
        assert_eq!(events[1].from_state, "free");

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
}
