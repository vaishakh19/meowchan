//! Diskwise — local-first disk space analyzer.
//!
//! The binary is a thin shell over this library, which is what makes the Rust half unit-testable
//! without a webview.
//!
//! Layering (nothing depends on something above it):
//!   `platform`   — Win32 entry points, declared by hand
//!   `filesystem` — long paths, metadata snapshots, category classification
//!   `scanner`    — the walker, aggregation, tree pruning, progress counters
//!   `duplicates` — size grouping, hashing, byte verification
//!   `cleanup`    — rules, findings, preview and execution
//!   `storage`    — SQLite: settings, scan history, cached entitlement, activations
//!   `licensing`  — signed payloads, offline entitlement resolution
//!   `config`     — the centralised commercial configuration
//!   `authz`      — the authorization layer every protected command goes through
//!   `commands`   — the narrowly-scoped Tauri command surface
//!
//! The frontend never talks to anything below `commands`, and `authz` is the only place a paid
//! capability is granted. A modified React interface cannot bypass it: the command itself calls
//! `authorize()` before doing anything.

pub mod authz;
pub mod cleanup;
pub mod commands;
pub mod config;
pub mod duplicates;
pub mod error;
pub mod filesystem;
pub mod licensing;
pub mod platform;
pub mod scanner;
#[cfg(test)]
pub mod security;
pub mod settings;
pub mod state;
pub mod storage;

pub use error::{AppError, AppResult};
pub use state::AppState;

use tauri::Manager;

/// Registers every command and managed state. Called by `main.rs`.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let state = AppState::new();

            // The database lives in the app's own data directory. Scanning works without it.
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let database_path = data_dir.join("diskwise.sqlite3");
            if let Ok(database) = crate::storage::db::Database::open(&database_path) {
                if let Ok(mut guard) = state.database.lock() {
                    *guard = Some(database);
                }
            }

            // Load persisted settings so the first scan honours them.
            if let Ok(settings) = crate::commands::settings::settings_get_for_bootstrap(&state) {
                if let Ok(mut guard) = state.settings.lock() {
                    *guard = Some(settings);
                }
            }

            // Resolve the entitlement from any stored licence key, entirely offline. A failure here
            // leaves the app on the free tier, which is a complete product.
            let device = crate::licensing::payload::DeviceBinding::current(&data_dir);
            let stored_key = state.stored_license_key().ok().flatten();
            let now = crate::licensing::verify::now_millis();
            let authorization = crate::authz::Authorization::resolve(
                stored_key.as_deref(),
                &crate::config::product::DEFAULT_CONFIG,
                now,
                device,
            );
            if let Ok(mut guard) = state.authorization.lock() {
                *guard = Some(authorization);
            }

            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::drives::list_drives,
            commands::scan::scan_start,
            commands::scan::scan_cancel,
            commands::scan::scan_get,
            commands::scan::scan_list,
            commands::scan::scan_children,
            commands::scan::scan_delete,
            commands::large_files::list_large_files,
            commands::files::file_details,
            commands::files::reveal_in_explorer,
            commands::files::open_path,
            commands::duplicates::find_duplicates,
            commands::duplicates::duplicates_get,
            commands::duplicates::verify_duplicates,
            commands::cleanup::cleanup_scan,
            commands::cleanup::cleanup_preview,
            commands::cleanup::cleanup_execute,
            commands::cleanup::recycle_bin_summary,
            commands::licensing::entitlement_get,
            commands::licensing::license_activate,
            commands::licensing::license_deactivate,
            commands::licensing::entitlement_events,
            commands::licensing::device_activations,
            commands::licensing::device_deactivate,
            commands::settings::settings_get,
            commands::settings::settings_update,
            commands::settings::privacy_report,
            commands::settings::app_version,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Diskwise");
}
