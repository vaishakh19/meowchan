//! Diskora Pro commands: status, checkout, activation and restore.
//!
//! The commands are the only bridge between the webview and the licensing
//! module. Verification always happens here, against Dodo Payments' public
//! license endpoints — never in the frontend.

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::licensing::{self, ProStatus};
use crate::state::AppState;

/// The licence state as of the last check, read from local storage
/// without any network round-trip. Fast enough for startup.
#[tauri::command]
pub fn pro_status(state: State<'_, AppState>) -> AppResult<ProStatus> {
    licensing::cached_status(&state)
}

/// Forces a fresh check with Dodo Payments. Used by the Pro page and
/// by "Restore purchases".
#[tauri::command]
pub async fn pro_verify(state: State<'_, AppState>) -> AppResult<ProStatus> {
    licensing::verified_status(&state).await
}

/// Opens the configured Dodo Payments checkout in the default browser
/// and returns the URL the user was sent to. The URL is a public
/// payment link — it is not a secret — but opening it stays a native
/// operation so the webview never needs network permissions.
#[tauri::command]
pub fn pro_checkout() -> AppResult<String> {
    let config = licensing::config();
    if !licensing::is_configured() {
        return Err(AppError::Licence(
            "This build of Diskora has no Dodo Payments configuration yet. The merchant must add a one-time product and payment link; see PRO-SETUP.md.".to_string(),
        ));
    }
    crate::platform::open_with_default_app(std::path::Path::new(&config.checkout_url))
        .map_err(AppError::NotPermitted)?;
    Ok(config.checkout_url)
}

/// Activates a license key the customer pasted in. Dodo verifies the key
/// server-side; Diskora additionally checks the key belongs to *this*
/// product before storing anything.
#[tauri::command]
pub async fn pro_activate(
    license_key: String,
    state: State<'_, AppState>,
) -> AppResult<ProStatus> {
    licensing::activate(&state, &license_key).await
}

/// Re-validates the stored purchase. This is the restore-purchases
/// flow: it works after a reinstall, a new machine profile, or a
/// corrupted local cache, because Dodo Payments remains the source
/// of truth.
#[tauri::command]
pub async fn pro_restore(state: State<'_, AppState>) -> AppResult<ProStatus> {
    licensing::restore(&state).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_licence_module_is_reachable_from_the_command_root() {
        // Compile-time smoke test: if `licensing` moves or is renamed,
        // these references break here instead of inside the handler macro.
        assert!(licensing::normalize_key("ABCD").is_some());
        assert!(licensing::normalize_key("   ").is_none());
        // The middle of a key is hidden; only its ends are shown.
        assert_eq!(licensing::mask_key("ABCD-1234-EFGH-5678"), "ABCD…5678");
        assert!(!licensing::mask_key("ABCD-1234-EFGH-5678").contains("EFGH"));
        assert!(!licensing::mask_key("ABCD-1234-EFGH-5678").contains("1234"));
    }

    #[test]
    fn the_status_contract_is_serializable_as_camel_case() {
        let status = ProStatus {
            enabled: false,
            configured: true,
            license_key: None,
            activated_at: None,
            last_validated_at: None,
            offline: false,
            message: "locked".to_string(),
        };
        let payload = serde_json::to_value(&status).unwrap();
        assert_eq!(payload["licenseKey"], serde_json::Value::Null);
        assert_eq!(payload["lastValidatedAt"], serde_json::Value::Null);
        assert_eq!(payload["enabled"], false);
    }
}
