//! Local storage.
//!
//! One SQLite database, in WAL mode, with `PRAGMA user_version` migrations. It holds preferences,
//! cached scan history and the cached entitlement. Nothing else: no file contents, no full-path
//! analytics, and the database can be deleted without affecting scanning.

pub mod db;

pub use db::{Database, SCHEMA_VERSION};
