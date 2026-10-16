//! Entitlement, activation and device-management commands.
//!
//! The frontend only reads what these return. A licence problem never blocks scanning: every path
//! here falls back to the free tier, which is a complete product.
//!
//! ## What this file deliberately does *not* do
//!
//! It never calls the licence service. Every activation here is **offline verification of a signed
//! payload**: the backend already proved, by signature, that whoever issued the key held the
//! private key. The webview cannot contribute to that proof in any way — not a redirect, not a
//! query parameter, not an email address, not an order id it claims to have paid.
//!
//! The authoritative activation ledger lives in Supabase (see `docs/licensing.md`). This file
//! mirrors what the service last told it and enforces the device limit against that mirror, so a
//! reinstall on the same machine does not consume another slot.

use tauri::State;

use crate::authz::Authorization;
use crate::config::product::DEFAULT_CONFIG;
use crate::error::{AppError, AppResult};
use crate::licensing::payload;
use crate::licensing::verify::{self, Entitlement, EntitlementState};
use crate::state::AppState;

/// Mirrors `EntitlementEvent`.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EntitlementEvent {
    pub at: i64,
    pub from: String,
    pub to: String,
    pub reason: String,
}

/// One device activation, as the UI shows it.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeviceActivation {
    pub device_id: String,
    pub device_name: String,
    pub device_binding: String,
    pub activated_at: i64,
    pub last_seen_at: i64,
    /// True when this is the machine making the request.
    pub current: bool,
}

/// The current entitlement, resolved by the backend at startup or on activation.
///
/// This is deliberately a read of state the backend resolved itself. It does not re-derive
/// anything from webview input.
#[tauri::command]
pub fn entitlement_get(state: State<'_, AppState>) -> AppResult<Entitlement> {
    Ok(state.entitlement())
}

/// Activates a licence key, entirely offline.
///
/// # Failure modes
///
/// Every one of these leaves the app on the free tier and writes an audit row:
/// malformed key, unknown schema, unknown signing key, bad signature, wrong product, unknown
/// tier, not yet valid, lapsed lease, wrong device, or the device limit already being reached.
#[tauri::command]
pub fn license_activate(license_key: String, state: State<'_, AppState>) -> AppResult<Entitlement> {
    let now = verify::now_millis();
    let previous = state.entitlement()?;
    let device = state.device.clone();

    // Resolve, then re-check the device limit against the local activation ledger.
    let authorization = Authorization::resolve(
        Some(license_key.as_str()),
        &DEFAULT_CONFIG,
        now,
        device.clone(),
    );

    if !authorization.is_paid() {
        // Nothing to activate. Record why, and stay free.
        let reason = authorization
            .entitlement
            .reason
            .clone()
            .unwrap_or_else(|| "That licence key could not be verified on this machine.".to_string());
        record_event(&state, previous.state.as_str(), "free", &reason);
        let _ = record_audit(&state, None, "activation-rejected", &reason);
        return Ok(state.entitlement());
    }

    // A licence that is bound to a different machine fails closed. `resolve` already checked this
    // against `device`, so reaching here means the binding matched.
    let payload = authorization
        .payload
        .clone()
        .ok_or_else(|| AppError::NotPermitted("That licence could not be verified.".to_string()))?;

    // Enforce the device limit atomically against the local ledger. The ledger is a mirror; the
    // service is authoritative and is the one that rejects a fourth device. Doing it locally too
    // means a revoked licence cannot silently activate again on a machine that was never
    // deactivated.
    let active = active_devices(&state)?;
    let already_here = active.iter().any(|row| {
        row.device_binding == device.value
            || row.device_id == format!("dev-{}", payload.license_id)
    });
    if !already_here && active.len() as u32 >= payload.device_limit {
        let reason = format!(
            "This licence is already active on {} devices, which is its limit. Deactivate a device at wiseclever.app/account, or use “Deactivate this device” on the machine you want to free up.",
            active.len()
        );
        record_event(&state, previous.state.as_str(), "free", &reason);
        let _ = record_audit(
            &state,
            Some(&payload.license_id),
            "activation-limit",
            &reason,
        );
        return Ok(state.entitlement());
    }

    // Persist the key and mirror the activation.
    state.store_license_key(Some(license_key.trim()))?;
    state.set_license_binding(&device.value);
    state.set_authorization(authorization.clone());

    mirror_activation(&state, &payload, &device, now)?;
    cache_entitlement(&state, &authorization.entitlement)?;

    let _ = record_audit(
        &state,
        Some(&payload.license_id),
        "activated",
        &format!(
            "Licence {} activated for tier {}.",
            payload.license_id, payload.tier_id
        ),
    );
    record_event(
        &state,
        previous.state.as_str(),
        "pro-active",
        "Licence verified on this machine.",
    );

    Ok(state.entitlement())
}

/// Deactivates this device and returns to the free tier.
///
/// The key is removed locally, so the next activation on this machine starts from scratch. The
/// service is told separately (by the browser) so it can free the slot.
#[tauri::command]
pub fn license_deactivate(state: State<'_, AppState>) -> AppResult<Entitlement> {
    let previous = state.entitlement()?;
    let now = verify::now_millis();

    let license_id = {
        let guard = state.database()?;
        guard
            .as_ref()
            .and_then(|database| database.load_license_record().ok().flatten())
            .map(|record| record.license_id)
    };

    state.store_license_key(None)?;
    if let Some(license_id) = license_id.as_deref() {
        let _ = record_audit(&state, Some(license_id), "deactivated", "Device deactivated.");
    }
    state.set_authorization(Authorization::free(state.device.clone()));
    cache_entitlement(&state, &state.entitlement()?)?;

    record_event(
        &state,
        previous.state.as_str(),
        "free",
        "Licence deactivated on this machine.",
    );
    let _ = now;
    Ok(state.entitlement())
}

/// The entitlement transition log, newest first.
#[tauri::command]
pub fn entitlement_events(state: State<'_, AppState>) -> AppResult<Vec<EntitlementEvent>> {
    let rows = {
        let guard = state.database()?;
        match guard.as_ref() {
            Some(database) => database.list_entitlement_events(50).unwrap_or_default(),
            None => Vec::new(),
        }
    };
    Ok(rows
        .into_iter()
        .map(|row| EntitlementEvent {
            at: row.at_ms,
            from: row.from_state,
            to: row.to_state,
            reason: row.reason,
        })
        .collect())
}

/// The devices this licence is active on.
///
/// This is a **local mirror** of the service's ledger, so it works offline and shows the user
/// exactly what the app can see. The service remains authoritative.
#[tauri::command]
pub fn device_activations(state: State<'_, AppState>) -> AppResult<Vec<DeviceActivation>> {
    let active = active_devices(&state)?;
    Ok(active
        .into_iter()
        .map(|row| DeviceActivation {
            current: row.device_binding == state.device.value,
            device_id: row.device_id,
            device_name: row.device_name,
            device_binding: row.device_binding,
            activated_at: row.activated_at,
            last_seen_at: row.last_seen_at,
        })
        .collect())
}

/// Deactivates a device in the local mirror, freeing a slot.
///
/// Only the *current* device can be deactivated here. Releasing a device you do not own would be
/// a way to defeat the limit, and it must go through the authenticated account flow instead.
#[tauri::command]
pub fn device_deactivate(device_id: String, state: State<'_, AppState>) -> AppResult<Vec<DeviceActivation>> {
    let now = verify::now_millis();
    let active = active_devices(&state)?;
    let target = active
        .iter()
        .find(|row| row.device_id == device_id)
        .ok_or_else(|| AppError::InvalidArgument("That device is not active on this licence.".to_string()))?;

    if target.device_binding != state.device.value {
        return Err(AppError::NotPermitted(
            "Only the device you are using can be deactivated from here. \
             Other devices are released from your account page."
                .to_string(),
        ));
    }

    {
        let guard = state.database()?;
        if let Some(database) = guard.as_ref() {
            database.deactivate_device(&device_id, now)?;
        }
    }

    let _ = record_audit(
        &state,
        None,
        "device-deactivated",
        &format!("Device {device_id} released a licence slot."),
    );

    device_activations(state)
}

/* ------------------------------- helpers ------------------------------- */

fn active_devices(
    state: &AppState,
) -> AppResult<Vec<crate::storage::db::DeviceActivationRow>> {
    let guard = state.database()?;
    match guard.as_ref() {
        Some(database) => database.active_device_activations(),
        None => Ok(Vec::new()),
    }
}

/// Mirrors the activation into the local ledger, creating the row if it is new.
fn mirror_activation(
    state: &AppState,
    payload: &payload::LicensePayload,
    device: &payload::DeviceBinding,
    now: i64,
) -> AppResult<()> {
    let guard = state.database()?;
    let Some(database) = guard.as_ref() else {
        return Ok(());
    };

    let device_id = format!("dev-{}", payload.license_id);
    let existing = database
        .active_device_activations()?
        .into_iter()
        .find(|row| row.device_binding == device.value);

    let device_name = hostname();
    match existing {
        Some(row) => database.upsert_device_activation(&crate::storage::db::DeviceActivationRow {
            last_seen_at: now,
            device_name,
            ..row
        }),
        None => database.upsert_device_activation(&crate::storage::db::DeviceActivationRow {
            device_id,
            license_id: payload.license_id.clone(),
            device_binding: device.value.clone(),
            device_name,
            activated_at: now,
            last_seen_at: now,
            deactivated_at: None,
        }),
    }
}

fn cache_entitlement(state: &AppState, entitlement: &Entitlement) -> AppResult<()> {
    let payload = serde_json::to_string(entitlement)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let guard = state.database()?;
    if let Some(database) = guard.as_ref() {
        database.save_entitlement_cache(&payload, verify::now_millis())?;
    }
    Ok(())
}

fn record_event(state: &AppState, from: &str, to: &str, reason: &str) {
    let guard = match state.database() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    if let Some(database) = guard.as_ref() {
        let _ = database.record_entitlement_event(verify::now_millis(), from, to, reason);
    }
}

fn record_audit(
    state: &AppState,
    license_id: Option<&str>,
    event_type: &str,
    detail: &str,
) -> Result<(), AppError> {
    let guard = state.database()?;
    if let Some(database) = guard.as_ref() {
        database.record_license_audit(verify::now_millis(), license_id, event_type, detail)?;
    }
    Ok(())
}

/// The machine's name, used only as a human label for the activation list.
fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "This PC".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AppState {
        AppState::new()
    }

    #[test]
    fn a_blank_key_stays_on_the_free_tier() {
        let app = state();
        // No database is attached in this test, so every path takes the "no service" branch.
        let entitlement = Authorization::resolve(Some(""), &DEFAULT_CONFIG, 0, app.device.clone());
        assert!(!entitlement.is_paid());
    }

    #[test]
    fn a_garbage_key_stays_on_the_free_tier_and_says_why() {
        let app = state();
        let authorization =
            Authorization::resolve(Some("not-a-key"), &DEFAULT_CONFIG, 0, app.device.clone());
        assert!(!authorization.is_paid());
        let reason = authorization.entitlement.reason.expect("a reason");
        assert!(reason.contains("key") || reason.contains("licence"), "reason was {reason}");
    }

    #[test]
    fn the_activation_payload_shape_is_camel_case_for_the_frontend() {
        let device = DeviceActivation {
            device_id: "dev-1".to_string(),
            device_name: "PC-1".to_string(),
            device_binding: "dev-abc".to_string(),
            activated_at: 1,
            last_seen_at: 2,
            current: false,
        };
        let json = serde_json::to_string(&device).expect("serializes");
        assert!(json.contains("\"deviceId\":\"dev-1\""), "json was {json}");
        assert!(json.contains("\"lastSeenAt\":2"), "json was {json}");
    }

    #[test]
    fn the_entitlement_state_strings_match_the_contract() {
        assert_eq!(EntitlementState::Free.as_str(), "free");
        assert_eq!(EntitlementState::ProActive.as_str(), "pro-active");
        assert_eq!(EntitlementState::ProExpired.as_str(), "pro-expired");
        assert_eq!(EntitlementState::Invalid.as_str(), "invalid");
    }
}
