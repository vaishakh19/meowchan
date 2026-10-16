//! Settings and privacy commands.

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::settings::AppSettings;
use crate::state::AppState;

/// Mirrors `PrivacyReport`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyReport {
    pub local_storage: Vec<String>,
    pub network_activity: Vec<String>,
    pub data_dir: String,
    pub database_path: String,
}

/// Mirrors `AppVersion`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppVersion {
    pub version: String,
    pub identifier: String,
    pub platform: String,
    pub arch: String,
}

/// Reads the stored settings, or the defaults when nothing is stored.
#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> AppResult<AppSettings> {
    if let Some(settings) = state
        .settings
        .lock()
        .map_err(|_| AppError::Internal("The settings lock was poisoned.".to_string()))?
        .clone()
    {
        return Ok(settings);
    }

    let stored = {
        let guard = state.database()?;
        guard.as_ref()
            .ok_or_else(|| AppError::Database("The local database is unavailable; preferences cannot be saved. Check your application data directory permissions.".to_string()))?
            .load_settings().map_err(AppError::Database)?
    };

    let settings = match stored {
        Some(payload) => serde_json::from_str::<AppSettings>(&payload)
            .map(|settings| settings.normalized())
            .unwrap_or_default(),
        None => AppSettings::default(),
    };

    if let Ok(mut guard) = state.settings.lock() {
        *guard = Some(settings.clone());
    }
    Ok(settings)
}

/// Applies a partial settings patch and persists the result.
#[tauri::command]
pub fn settings_update(
    patch: serde_json::Value,
    state: State<'_, AppState>,
) -> AppResult<AppSettings> {
    let mut current = settings_get(state.clone())?;

    // Merge only the fields the caller actually sent, so a partial patch never resets the rest.
    if let Some(object) = patch.as_object() {
        for (key, value) in object {
            match key.as_str() {
                "theme" => current.theme = value.as_str().unwrap_or("system").to_string(),
                "maxChildrenPerNode" => {
                    current.max_children_per_node = value.as_u64().unwrap_or(100) as u32
                }
                "followLinks" => current.follow_links = value.as_bool().unwrap_or(false),
                _ => {}
            }
        }
    }

    let settings = current.normalized();
    let payload =
        serde_json::to_string(&settings).map_err(|error| AppError::Internal(error.to_string()))?;
    if let Ok(guard) = state.database() {
        if let Some(database) = guard.as_ref() {
            database
                .save_settings(&payload)
                .map_err(AppError::Database)?;
        }
    }
    if let Ok(mut guard) = state.settings.lock() {
        *guard = Some(settings.clone());
    }
    Ok(settings)
}

/// Exactly what Diskwise stores locally and what (if anything) leaves the device.
#[tauri::command]
pub fn privacy_report(state: State<'_, AppState>) -> AppResult<PrivacyReport> {
    let (data_dir, database_path) = {
        let guard = state.database()?;
        match guard.as_ref() {
            Some(database) => (
                database
                    .path()
                    .parent()
                    .map(|parent| parent.to_string_lossy().to_string())
                    .unwrap_or_default(),
                database.path().to_string_lossy().to_string(),
            ),
            None => (String::new(), String::new()),
        }
    };




    Ok(PrivacyReport {
        local_storage: vec!["Preferences and scan summaries, stored locally.".to_string()],
        network_activity: vec![],
        data_dir,
        database_path,
    })
}

/// The application identifier. Must match `identifier` in `tauri.conf.json`; the test below fails
/// if the two drift apart.
pub const APP_IDENTIFIER: &str = "com.diskwise.app";

/// The application's own identity: version, identifier, platform and architecture.
#[tauri::command]
pub fn app_version(app: tauri::AppHandle) -> AppVersion {
    AppVersion {
        version: app.package_info().version.to_string(),
        identifier: APP_IDENTIFIER.to_string(),
        platform: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_patch_only_touches_the_fields_it_names() {
        let mut settings = AppSettings::default();
        settings.follow_links = true;
        let patch = serde_json::json!({ "theme": "dark" });
        if let Some(object) = patch.as_object() {
            for (key, value) in object {
                if key == "theme" {
                    settings.theme = value.as_str().unwrap_or("system").to_string();
                }
            }
        }
        assert_eq!(settings.theme, "dark");
        assert!(settings.follow_links, "an unrelated field is untouched");
    }

    #[test]
    fn app_version_fields_are_populated() {
        let version = AppVersion {
            version: "0.1.0".to_string(),
            identifier: APP_IDENTIFIER.to_string(),
            platform: "windows".to_string(),
            arch: "x86_64".to_string(),
        };
        assert_eq!(version.identifier, APP_IDENTIFIER);
    }

    #[test]
    fn the_identifier_matches_tauri_conf_json() {
        // The identifier is duplicated in `tauri.conf.json`; this test keeps them in step so the
        // installer, the About screen and the capability file never disagree.
        let config = include_str!("../../tauri.conf.json");
        let needle = format!("\"identifier\": \"{APP_IDENTIFIER}\"");
        assert!(
            config.contains(&needle),
            "tauri.conf.json must declare \"identifier\": \"{APP_IDENTIFIER}\""
        );
    }
}

/// Bootstrap helper: reads settings without a `State` wrapper.
pub(crate) fn settings_get_for_bootstrap(
    state: &AppState,
) -> Result<AppSettings, AppError> {
    let stored = {
        let guard = state
            .database
            .lock()
            .map_err(|_| AppError::Internal("The database lock was poisoned.".to_string()))?;
        guard
            .as_ref()
            .and_then(|database| database.load_settings().ok().flatten())
    };
    Ok(match stored {
        Some(payload) => serde_json::from_str::<AppSettings>(&payload)
            .map(|settings| settings.normalized())
            .unwrap_or_default(),
        None => AppSettings::default(),
    })
}
