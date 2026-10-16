//! Diskora Pro licensing, verified against Dodo Payments.
//!
//! Dodo Payments exposes its license activation and validation endpoints as *public*
//! endpoints: they require no API key and are designed to be called directly from
//! desktop software. That is what makes the smallest secure integration possible:
//!
//! * The merchant API key — the only secret in the Dodo world — never leaves the
//!   merchant dashboard. It is not needed to activate or validate a key, so it cannot
//!   leak through the executable or the webview bundle.
//! * Activation and validation are authoritative checks against Dodo's own database.
//!   A user cannot forge a purchase by editing a local file: every unlock decision
//!   starts from a server-side validation (with a short, documented offline grace
//!   period so a spotty connection does not lock a paying customer out).
//! * The product is a one-time purchase. It is configured as a one-time price with
//!   license keys in the Dodo dashboard; nothing here creates or renews a subscription.
//!
//! Configuration is build-time (`option_env!`), documented in PRO-SETUP.md:
//!   `DISKORA_DODO_PRODUCT_ID`    — the one-time product id, e.g. `pdt_…`   (required)
//!   `DISKORA_DODO_CHECKOUT_URL`  — the product's hosted payment link       (required)
//!   `DISKORA_DODO_ENVIRONMENT`   — `live` (default) or `test`
//!
//! When the configuration is missing the app reports "not configured" instead of
//! pretending a payment flow exists.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// How long a previously verified entitlement stays trusted when Dodo Payments
/// cannot be reached. A paying user with a broken connection keeps Pro for this
/// long; every online check re-verifies immediately.
pub const OFFLINE_GRACE_MS: i64 = 7 * 24 * 60 * 60 * 1000;

const NOT_CONFIGURED: &str = "This build of Diskora has no Dodo Payments configuration. The merchant must set a one-time product and a payment link before Diskora Pro can be sold; see PRO-SETUP.md.";
const NOT_PURCHASED: &str = "Diskora Pro is not active on this device. Buy it once from the Diskora Pro page to unlock safe deletion and full paths.";

/* ------------------------------- configuration ------------------------------- */

#[derive(Debug, Clone)]
pub struct LicensingConfig {
    /// Dodo Payments API base, e.g. `https://live.dodopayments.com`.
    pub api_base: String,
    /// The one-time product this app sells, e.g. `pdt_…`.
    pub product_id: String,
    /// Hosted payment link for the product (created in the Dodo dashboard).
    pub checkout_url: String,
    /// `live` or `test`.
    pub environment: &'static str,
}

/// Build-time configuration. Values come from the environment the app was compiled in,
/// so no secret and no URL is invented at runtime.
pub fn config() -> LicensingConfig {
    let environment = match option_env!("DISKORA_DODO_ENVIRONMENT") {
        Some("test") | Some("test_mode") | Some("TEST") => "test",
        _ => "live",
    };
    let api_base = match option_env!("DISKORA_DODO_API_BASE") {
        Some(custom) if !custom.trim().is_empty() => custom.trim().to_string(),
        _ if environment == "test" => "https://test.dodopayments.com".to_string(),
        _ => "https://live.dodopayments.com".to_string(),
    };
    LicensingConfig {
        api_base,
        product_id: option_env!("DISKORA_DODO_PRODUCT_ID")
            .unwrap_or_default()
            .trim()
            .to_string(),
        checkout_url: option_env!("DISKORA_DODO_CHECKOUT_URL")
            .unwrap_or_default()
            .trim()
            .to_string(),
        environment,
    }
}

/// True when the build carries the merchant's product id and payment link. Without
/// these, no purchase is possible and the UI says so instead of faking a checkout.
pub fn is_configured() -> bool {
    let config = config();
    !config.product_id.is_empty() && !config.checkout_url.is_empty()
}

/* -------------------------------- entitlement -------------------------------- */

/// A verified purchase, persisted locally so Pro survives restarts. The stored key is
/// only an *input* to future verification — it never unlocks anything on its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredEntitlement {
    pub license_key: String,
    /// Dodo's activation instance id (`lki_…`), tied to this device.
    pub instance_id: Option<String>,
    /// Product the key was validated against, e.g. `pdt_…`.
    pub product_id: String,
    /// Stable per-device label sent at activation ("Diskora on WORKSTATION").
    pub device_name: String,
    pub activated_at: i64,
    pub last_validated_at: i64,
}

/// What the app may do, as decided by the licensing module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProDecision {
    /// A stored entitlement was just re-verified with Dodo Payments.
    Verified(StoredEntitlement),
    /// Dodo was unreachable, but this entitlement was verified recently enough.
    OfflineGrace(StoredEntitlement),
    /// No purchase, or the purchase is no longer valid.
    Locked,
}

impl ProDecision {
    pub fn is_active(&self) -> bool {
        !matches!(self, ProDecision::Locked)
    }

    pub fn entitlement(&self) -> Option<&StoredEntitlement> {
        match self {
            ProDecision::Verified(entitlement) | ProDecision::OfflineGrace(entitlement) => {
                Some(entitlement)
            }
            ProDecision::Locked => None,
        }
    }
}

/// The single decision rule. Pure on purpose so the grace-period behaviour is tested
/// without any network: a fresh server-side `valid=true` always unlocks; an unreachable
/// server only unlocks inside the grace window; anything else stays locked.
pub fn decide(
    stored: Option<StoredEntitlement>,
    server_says_valid: Option<bool>,
    now_ms: i64,
) -> ProDecision {
    match (stored, server_says_valid) {
        (Some(entitlement), Some(true)) => ProDecision::Verified(entitlement),
        (Some(entitlement), None)
            if now_ms.saturating_sub(entitlement.last_validated_at) <= OFFLINE_GRACE_MS =>
        {
            ProDecision::OfflineGrace(entitlement)
        }
        _ => ProDecision::Locked,
    }
}

/// True when the stored entitlement was verified recently enough to trust offline.
pub fn within_grace(entitlement: &StoredEntitlement, now_ms: i64) -> bool {
    now_ms.saturating_sub(entitlement.last_validated_at) <= OFFLINE_GRACE_MS
}

/* ------------------------------- presentation ------------------------------- */

/// What the UI shows about the licence. Machine decisions only — no marketing copy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProStatus {
    pub enabled: bool,
    pub configured: bool,
    /// Masked key (`ABCD…WXYZ`), when an entitlement is stored.
    pub license_key: Option<String>,
    pub activated_at: Option<i64>,
    pub last_validated_at: Option<i64>,
    /// True when the active state rests on the offline grace period right now.
    pub offline: bool,
    pub message: String,
}

impl ProStatus {
    fn locked(configured: bool, message: String) -> Self {
        Self {
            enabled: false,
            configured,
            license_key: None,
            activated_at: None,
            last_validated_at: None,
            offline: false,
            message,
        }
    }
}

pub fn status_for(decision: &ProDecision) -> ProStatus {
    let configured = is_configured();
    match decision {
        ProDecision::Locked => {
            let message = if configured {
                NOT_PURCHASED.to_string()
            } else {
                NOT_CONFIGURED.to_string()
            };
            ProStatus::locked(configured, message)
        }
        ProDecision::OfflineGrace(entitlement) => ProStatus {
            enabled: true,
            configured,
            license_key: Some(mask_key(&entitlement.license_key)),
            activated_at: Some(entitlement.activated_at),
            last_validated_at: Some(entitlement.last_validated_at),
            offline: true,
            message: "Diskora Pro is active. Dodo Payments was unreachable, so the last verified purchase is being trusted until the connection returns.".to_string(),
        },
        ProDecision::Verified(entitlement) => ProStatus {
            enabled: true,
            configured,
            license_key: Some(mask_key(&entitlement.license_key)),
            activated_at: Some(entitlement.activated_at),
            last_validated_at: Some(entitlement.last_validated_at),
            offline: false,
            message: "Diskora Pro is active. One-time purchase; no subscription.".to_string(),
        },
    }
}

/// `ABCD-…-WXYZ` — enough to recognise a key, never enough to use it.
pub fn mask_key(key: &str) -> String {
    let characters: Vec<char> = key.trim().chars().collect();
    if characters.len() <= 8 {
        return "••••".to_string();
    }
    let head: String = characters.iter().take(4).collect();
    let tail: String = characters.iter().skip(characters.len() - 4).collect();
    format!("{head}…{tail}")
}

/// Accepts a pasted key: trims it, rejects anything empty, oversized or control-coded.
pub fn normalize_key(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 256 {
        return None;
    }
    if !trimmed.chars().all(|character| character.is_ascii() && !character.is_control()) {
        return None;
    }
    Some(trimmed.to_string())
}

/// Stable per-device label for the Dodo activation instance. Display-only; it never
/// identifies the user beyond what the OS already exposes.
pub fn device_name() -> String {
    let host = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    let name = if host.trim().is_empty() {
        "Diskora on this device".to_string()
    } else {
        format!("Diskora on {host}")
    };
    name.chars().take(64).collect()
}

/* ------------------------------ Dodo Payments ------------------------------ */

#[derive(Deserialize)]
struct DodoActivateResponse {
    id: Option<String>,
    product: Option<DodoProduct>,
}

#[derive(Deserialize)]
struct DodoProduct {
    product_id: Option<String>,
}

#[derive(Deserialize)]
struct DodoValidateResponse {
    valid: Option<bool>,
}

/// A successful activation: Dodo's instance id and the product the key belongs to.
pub struct Activation {
    pub instance_id: Option<String>,
    pub product_id: String,
}

/// Thin client over Dodo Payments' *public* license endpoints. No API key is used or
/// stored anywhere in Diskora — these endpoints exist precisely so client software can
/// verify keys without shipping merchant credentials.
pub struct DodoClient {
    client: reqwest::Client,
    api_base: String,
}

impl DodoClient {
    /// Builds the HTTP client. Falls back to no proxy configuration
    /// errors: a client that cannot be built is reported, never
    /// silently defaulted.
    pub fn new(api_base: String) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| format!("Could not create the HTTP client: {error}"))?;
        Ok(Self { client, api_base })
    }

    /// Activates `license_key` on this device. Dodo enforces the product's activation
    /// limit and key validity; Diskora only interprets the answer.
    pub async fn activate(&self, license_key: &str, device: &str) -> Result<Activation, String> {
        let response = self
            .client
            .post(format!("{}/licenses/activate", self.api_base))
            .json(&serde_json::json!({ "license_key": license_key, "name": device }))
            .send()
            .await
            .map_err(|error| format!("Could not reach Dodo Payments: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("Dodo Payments sent an unreadable response: {error}"))?;
        if !status.is_success() {
            return Err(activation_failure(status.as_u16(), &body));
        }
        let parsed: DodoActivateResponse = serde_json::from_str(&body)
            .map_err(|_| "Dodo Payments returned an activation result Diskora could not read.".to_string())?;
        Ok(Activation {
            instance_id: parsed.id,
            product_id: parsed
                .product
                .and_then(|product| product.product_id)
                .unwrap_or_default(),
        })
    }

    /// Checks a key (optionally pinned to this device's activation instance).
    pub async fn validate(&self, license_key: &str, instance_id: Option<&str>) -> Result<bool, String> {
        let response = self
            .client
            .post(format!("{}/licenses/validate", self.api_base))
            .json(&serde_json::json!({
                "license_key": license_key,
                "license_key_instance_id": instance_id,
            }))
            .send()
            .await
            .map_err(|error| format!("Could not reach Dodo Payments: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("Dodo Payments sent an unreadable response: {error}"))?;
        if !status.is_success() {
            // A key that is unknown, revoked, expired or exhausted is simply invalid.
            return Ok(false);
        }
        let parsed: DodoValidateResponse = serde_json::from_str(&body)
            .map_err(|_| "Dodo Payments returned a validation result Diskora could not read.".to_string())?;
        Ok(parsed.valid.unwrap_or(false))
    }
}

/// Maps Dodo's error answers onto one honest sentence for the user.
fn activation_failure(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(|field| field.as_str())
                .or_else(|| value.get("detail").and_then(|field| field.as_str()))
                .or_else(|| {
                    value
                        .get("error")
                        .and_then(|error| error.get("message"))
                        .and_then(|field| field.as_str())
                })
                .map(str::to_string)
        });
    match (status, detail.as_deref()) {
        (401, _) | (403, _) => "Dodo Payments rejected this key. Check it and try again.".to_string(),
        (404, _) => "Dodo Payments does not know this license key.".to_string(),
        (409, _) | (422, _) => format!(
            "This license key cannot be activated: {}. Restore purchases on a device that is already activated, or contact support.",
            detail.as_deref().unwrap_or("the activation limit may be reached")
        ),
        (status, Some(detail)) => format!("Dodo Payments rejected the activation (HTTP {status}): {detail}"),
        (status, None) => format!("Dodo Payments rejected the activation (HTTP {status})."),
    }
}

/* ------------------------------ app integration ------------------------------ */

/// Reads the stored entitlement, if any, without any network access.
fn stored_entitlement(state: &AppState) -> AppResult<Option<StoredEntitlement>> {
    let guard = state.database()?;
    let database = guard
        .as_ref()
        .ok_or_else(|| AppError::Database("The local database is unavailable; Diskora Pro cannot be activated.".to_string()))?;
    let payload = database.load_entitlement().map_err(AppError::Database)?;
    match payload {
        Some(text) => serde_json::from_str::<StoredEntitlement>(&text)
            .map(Some)
            .map_err(|error| AppError::Database(format!("The stored entitlement is unreadable: {error}"))),
        None => Ok(None),
    }
}

/// The startup status: fast, honest, and never more optimistic than the last
/// verified check plus the offline grace window.
pub fn cached_status(state: &AppState) -> AppResult<ProStatus> {
    let decision = decide(stored_entitlement(state)?, None, crate::time::now_millis());
    Ok(status_for(&decision))
}

/// Revalidates with Dodo and refreshes the stored entitlement. Used by the Pro page,
/// by restore-purchases, and — critically — by safe deletion before it runs.
pub async fn verified_status(state: &AppState) -> AppResult<ProStatus> {
    let decision = verified_decision(state).await?;
    Ok(status_for(&decision))
}

/// The decision safe deletion gates on: a fresh server check, falling back to the
/// grace window only when Dodo Payments is unreachable.
pub async fn verified_decision(state: &AppState) -> AppResult<ProDecision> {
    let Some(entitlement) = stored_entitlement(state)? else {
        return Ok(ProDecision::Locked);
    };
    let client = DodoClient::new(config().api_base).map_err(AppError::Licence)?;
    let validation = client
        .validate(&entitlement.license_key, entitlement.instance_id.as_deref())
        .await;
    match validation {
        Ok(true) => {
            touch_validated(state, &entitlement)?;
            Ok(ProDecision::Verified(entitlement))
        }
        Ok(false) => {
            // The key was revoked, refunded, expired or exhausted — stop trusting it.
            clear_stored(state)?;
            Ok(ProDecision::Locked)
        }
        Err(network) => {
            if within_grace(&entitlement, crate::time::now_millis()) {
                Ok(ProDecision::OfflineGrace(entitlement))
            } else {
                Err(AppError::Licence(format!(
                    "Diskora could not verify your Pro purchase ({network}). Reconnect and use Restore purchases."
                )))
            }
        }
    }
}

/// Requires an active Pro entitlement, or an error the command layer surfaces.
pub async fn require_pro(state: &AppState) -> AppResult<()> {
    if verified_decision(state).await?.is_active() {
        Ok(())
    } else {
        Err(AppError::Licence(NOT_PURCHASED.to_string()))
    }
}

/// Activates a pasted key: Dodo checks it, Diskora checks it belongs to *this* product,
/// then the entitlement is persisted for offline grace use.
pub async fn activate(state: &AppState, license_key: &str) -> AppResult<ProStatus> {
    if !is_configured() {
        return Err(AppError::Licence(NOT_CONFIGURED.to_string()));
    }
    let key = normalize_key(license_key)
        .ok_or_else(|| AppError::InvalidArgument("Enter the license key exactly as Dodo Payments emailed it.".to_string()))?;
    let device = device_name();
    let client = DodoClient::new(config().api_base).map_err(AppError::Licence)?;
    let activation = client
        .activate(&key, &device)
        .await
        .map_err(AppError::Licence)?;

    // A key for some other merchant's product must never unlock Diskora Pro.
    if activation.product_id != config().product_id {
        let _ = clear_stored(state);
        return Err(AppError::Licence(
            "This license key belongs to a different product. Diskora Pro keys come from the Diskora Pro page.".to_string(),
        ));
    }

    let now = crate::time::now_millis();
    let entitlement = StoredEntitlement {
        license_key: key,
        instance_id: activation.instance_id,
        product_id: activation.product_id,
        device_name: device,
        activated_at: now,
        last_validated_at: now,
    };
    save_stored(state, &entitlement)?;
    Ok(status_for(&ProDecision::Verified(entitlement)))
}

/// Forces a fresh check of whatever is stored. This is the restore-purchases flow:
/// it re-validates the key with Dodo and refreshes the grace window.
pub async fn restore(state: &AppState) -> AppResult<ProStatus> {
    if !is_configured() {
        return Err(AppError::Licence(NOT_CONFIGURED.to_string()));
    }
    Ok(status_for(&verified_decision(state).await?))
}

fn touch_validated(state: &AppState, entitlement: &StoredEntitlement) -> AppResult<()> {
    let mut refreshed = entitlement.clone();
    refreshed.last_validated_at = crate::time::now_millis();
    save_stored(state, &refreshed)
}

fn save_stored(state: &AppState, entitlement: &StoredEntitlement) -> AppResult<()> {
    let payload = serde_json::to_string(entitlement)
        .map_err(|error| AppError::Internal(format!("The entitlement could not be serialized: {error}")))?;
    let guard = state.database()?;
    let database = guard
        .as_ref()
        .ok_or_else(|| AppError::Database("The local database is unavailable; Diskora Pro cannot be activated.".to_string()))?;
    database.save_entitlement(&payload).map_err(AppError::Database)
}

fn clear_stored(state: &AppState) -> AppResult<()> {
    let guard = state.database()?;
    if let Some(database) = guard.as_ref() {
        database.clear_entitlement().map_err(AppError::Database)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entitlement(last_validated_at: i64) -> StoredEntitlement {
        StoredEntitlement {
            license_key: "ABCD-1234-EFGH-5678".to_string(),
            instance_id: Some("lki_123".to_string()),
            product_id: "pdt_test".to_string(),
            device_name: "Diskora on TEST-PC".to_string(),
            activated_at: 1_000,
            last_validated_at,
        }
    }

    const NOW: i64 = 10_000_000_000;

    #[test]
    fn a_fresh_server_verification_always_unlocks() {
        let decision = decide(Some(entitlement(0)), Some(true), NOW);
        assert_eq!(decision, ProDecision::Verified(entitlement(0)));
    }

    #[test]
    fn a_rejected_key_never_unlocks_even_within_the_grace_window() {
        let decision = decide(Some(entitlement(NOW)), Some(false), NOW);
        assert_eq!(decision, ProDecision::Locked);
    }

    #[test]
    fn an_unverified_key_stays_locked_even_online() {
        assert_eq!(decide(None, Some(true), NOW), ProDecision::Locked);
        assert_eq!(decide(None, None, NOW), ProDecision::Locked);
    }

    #[test]
    fn offline_grace_only_covers_recently_verified_entitlements() {
        let fresh = decide(Some(entitlement(NOW - 1000)), None, NOW);
        assert_eq!(fresh, ProDecision::OfflineGrace(entitlement(NOW - 1000)));

        let stale = decide(Some(entitlement(NOW - OFFLINE_GRACE_MS - 1)), None, NOW);
        assert_eq!(stale, ProDecision::Locked);
    }

    #[test]
    fn grace_boundary_is_exact() {
        assert_eq!(
            decide(Some(entitlement(NOW - OFFLINE_GRACE_MS)), None, NOW),
            ProDecision::OfflineGrace(entitlement(NOW - OFFLINE_GRACE_MS))
        );
    }

    #[test]
    fn status_reports_the_decision_honestly() {
        let active = status_for(&ProDecision::Verified(entitlement(NOW)));
        assert!(active.enabled);
        assert_eq!(active.license_key.as_deref(), Some("ABCD…5678"));
        assert!(!active.offline);

        let offline = status_for(&ProDecision::OfflineGrace(entitlement(NOW)));
        assert!(offline.enabled);
        assert!(offline.offline);

        let locked = status_for(&ProDecision::Locked);
        assert!(!locked.enabled);
        assert_eq!(locked.license_key, None);
    }

    #[test]
    fn license_keys_are_masked_not_hidden_or_leaked() {
        assert_eq!(mask_key("ABCD-1234-EFGH-5678"), "ABCD…5678");
        assert_eq!(mask_key("short"), "••••");
        assert_eq!(mask_key("12345678"), "••••");
        assert_eq!(mask_key("123456789"), "1234…6789");
    }

    #[test]
    fn keys_are_normalized_and_reject_nonsense() {
        assert_eq!(normalize_key("  ABCD-1234 \n"), Some("ABCD-1234".to_string()));
        assert_eq!(normalize_key("   "), None);
        assert_eq!(normalize_key(&"x".repeat(257)), None);
        assert_eq!(normalize_key("bad\x01key"), None);
    }

    #[test]
    fn activation_failures_map_to_honest_messages() {
        assert!(activation_failure(404, "{}").contains("does not know"));
        assert!(activation_failure(409, r#"{"message":"limit reached"}"#).contains("limit reached"));
        assert!(activation_failure(500, "boom").contains("HTTP 500"));
    }

    #[test]
    fn device_names_are_bounded_and_never_empty_of_context() {
        let name = device_name();
        assert!(!name.is_empty());
        assert!(name.len() <= 64);
    }
}
