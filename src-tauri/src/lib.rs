//! Diskora: local disk analysis. Free features are read-only; the two Pro
//! capabilities (guarded Recycle Bin deletion and full-path display) are gated
//! by Dodo Payments licence verification in `licensing` and `commands::pro`.
pub mod commands;
pub mod duplicates;
pub mod error;
pub mod filesystem;
pub mod licensing;
pub mod platform;
pub mod scanner;
pub mod settings;
pub mod state;
pub mod storage;
mod time;

pub use error::{AppError, AppResult};
pub use state::AppState;

use tauri::Manager;

/// Registers every command and managed state. Called by `main.rs`.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = AppState::new();

            // The database lives in the app's own data directory. Scanning works without it.
            let data_dir = app.path().app_data_dir()?;
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

            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::drives::list_drives,
            commands::drives::storage_breakdown,
            commands::scan::scan_start,
            commands::scan::scan_cancel,
            commands::scan::scan_get,
            commands::scan::scan_list,
            commands::scan::scan_children,
            commands::scan::scan_delete,
            commands::scan::user_storage_report,
            commands::scan::export_scan_csv,
            commands::scan::compare_scans,
            commands::large_files::list_large_files,
            commands::files::file_details,
            commands::files::reveal_in_explorer,
            commands::files::open_path,
            commands::duplicates::find_duplicates,
            commands::duplicates::duplicates_get,
            commands::duplicates::verify_duplicates,
            commands::settings::settings_get,
            commands::settings::settings_update,
            commands::settings::privacy_report,
            commands::settings::app_version,
            commands::pro::pro_status,
            commands::pro::pro_verify,
            commands::pro::pro_checkout,
            commands::pro::pro_activate,
            commands::pro::pro_restore,
            commands::delete::delete_items,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Diskora");
}
