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
const NOT_PURCHASED: &str = "Diskora Pro is not active on this device. Buy it once ($12.99, no subscription) to unlock Delete to Recycle Bin, Reveal in Explorer and full paths.";
const REVOKED: &str = "This Diskora Pro licence is no longer valid. It may have been revoked, refunded, expired or used on too many devices. Check the key, or contact support.";
const UNVERIFIED: &str = "Your Diskora Pro purchase has not been re-verified for more than 7 days. Connect to the internet and press Restore purchases.";

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
        api_base: api_base.trim_end_matches('/').to_string(),
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

fn https_host(url: &str) -> Option<&str> {
    let remainder = url.strip_prefix("https://")?;
    let authority_end = remainder
        .find(|character| matches!(character, '/' | '?' | '#'))
        .unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    if authority.is_empty()
        || authority.chars().any(|character| matches!(character, '@' | ':' | '\\'))
        || !authority.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return None;
    }
    Some(authority)
}

fn api_base_is_allowlisted(base: &str) -> bool {
    let normalized = base.trim_end_matches('/');
    ["https://live.dodopayments.com", "https://test.dodopayments.com"]
        .iter()
        .any(|allowed| normalized.eq_ignore_ascii_case(allowed))
}

fn config_hosts_are_allowlisted(config: &LicensingConfig) -> bool {
    let expected_api = if config.environment == "test" {
        "test.dodopayments.com"
    } else {
        "live.dodopayments.com"
    };
    let checkout_host = https_host(&config.checkout_url);
    let expected_checkout = ["checkout.dodopayments.com", "test.checkout.dodopayments.com"];
    https_host(&config.api_base).is_some_and(|host| host.eq_ignore_ascii_case(expected_api))
        && api_base_is_allowlisted(&config.api_base)
        && checkout_host.is_some_and(|host| expected_checkout.iter().any(|allowed| host.eq_ignore_ascii_case(allowed)))
}

/// True when the build carries a product id and Dodo-hosted checkout/API URLs.
/// Only those allowlisted HTTPS endpoints can be used by the licensing path.
pub fn is_configured() -> bool {
    let config = config();
    !config.product_id.is_empty()
        && !config.checkout_url.is_empty()
        && config_hosts_are_allowlisted(&config)
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

const PROTECTED_ENTITLEMENT_PREFIX: &str = "dpapi-v1:";
const MAX_ENTITLEMENT_BYTES: usize = 16 * 1024;

/// Protects the entire entitlement JSON with user-scoped Windows DPAPI before it reaches SQLite.
fn protect_entitlement(json: &str) -> Result<String, String> {
    if json.len() > MAX_ENTITLEMENT_BYTES {
        return Err("The entitlement exceeds the supported size.".to_string());
    }
    let protected = dpapi_protect(json.as_bytes())?;
    Ok(format!("{PROTECTED_ENTITLEMENT_PREFIX}{}", encode_hex(&protected)))
}

/// Returns `None` for a legacy plaintext row so the caller can migrate it in place.
fn unprotect_entitlement(stored: &str) -> Result<Option<String>, String> {
    if stored.len() > MAX_ENTITLEMENT_BYTES * 2 + PROTECTED_ENTITLEMENT_PREFIX.len() {
        return Err("The stored entitlement exceeds the supported size.".to_string());
    }
    let Some(hex) = stored.strip_prefix(PROTECTED_ENTITLEMENT_PREFIX) else {
        return Ok(None);
    };
    if hex.len() > MAX_ENTITLEMENT_BYTES * 2 {
        return Err("The protected entitlement exceeds the supported size.".to_string());
    }
    let encrypted = decode_hex(hex)?;
    let bytes = dpapi_unprotect(&encrypted)?;
    let json = String::from_utf8(bytes)
        .map_err(|_| "The protected entitlement is not valid UTF-8.".to_string())?;
    Ok(Some(json))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    if text.len() % 2 != 0 || text.len() > MAX_ENTITLEMENT_BYTES * 2 {
        return Err("The protected entitlement encoding is malformed.".to_string());
    }
    let mut decoded = Vec::with_capacity(text.len() / 2);
    for pair in text.as_bytes().chunks_exact(2) {
        let high = (pair[0] as char).to_digit(16);
        let low = (pair[1] as char).to_digit(16);
        match (high, low) {
            (Some(high), Some(low)) => decoded.push(((high << 4) | low) as u8),
            _ => return Err("The protected entitlement encoding is malformed.".to_string()),
        }
    }
    Ok(decoded)
}

#[cfg(windows)]
#[repr(C)]
struct DataBlob {
    cb_data: u32,
    pb_data: *mut u8,
}

#[cfg(windows)]
#[link(name = "Crypt32")]
unsafe extern "system" {
    #[link_name = "CryptProtectData"]
    fn crypt_protect_data(
        data_in: *const DataBlob,
        description: *const u16,
        optional_entropy: *const DataBlob,
        reserved: *mut std::ffi::c_void,
        prompt: *mut std::ffi::c_void,
        flags: u32,
        data_out: *mut DataBlob,
    ) -> i32;
    #[link_name = "CryptUnprotectData"]
    fn crypt_unprotect_data(
        data_in: *const DataBlob,
        description: *mut *mut u16,
        optional_entropy: *const DataBlob,
        reserved: *mut std::ffi::c_void,
        prompt: *mut std::ffi::c_void,
        flags: u32,
        data_out: *mut DataBlob,
    ) -> i32;
}

#[cfg(windows)]
#[link(name = "Kernel32")]
unsafe extern "system" {
    #[link_name = "LocalFree"]
    fn local_free(memory: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
}

#[cfg(windows)]
fn dpapi_protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    let length = u32::try_from(plain.len()).map_err(|_| "The entitlement is too large to protect.".to_string())?;
    let input = DataBlob { cb_data: length, pb_data: plain.as_ptr() as *mut u8 };
    let mut output = DataBlob { cb_data: 0, pb_data: std::ptr::null_mut() };
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
    // SAFETY: both blobs point to valid buffers for the duration of the call; the optional
    // description, entropy, prompt, and reserved parameters are intentionally null.
    let ok = unsafe {
        crypt_protect_data(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(format!("Windows DPAPI could not protect the entitlement: {}", std::io::Error::last_os_error()));
    }
    take_dpapi_output(output)
}

#[cfg(windows)]
fn dpapi_unprotect(encrypted: &[u8]) -> Result<Vec<u8>, String> {
    let length = u32::try_from(encrypted.len()).map_err(|_| "The protected entitlement is too large.".to_string())?;
    let input = DataBlob { cb_data: length, pb_data: encrypted.as_ptr() as *mut u8 };
    let mut output = DataBlob { cb_data: 0, pb_data: std::ptr::null_mut() };
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
    // SAFETY: the input blob is valid for the call and Windows owns/allocates the output blob.
    let ok = unsafe {
        crypt_unprotect_data(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(format!("Windows DPAPI could not unprotect the entitlement: {}", std::io::Error::last_os_error()));
    }
    take_dpapi_output(output)
}

#[cfg(windows)]
fn take_dpapi_output(output: DataBlob) -> Result<Vec<u8>, String> {
    if output.cb_data > 0 && output.pb_data.is_null() {
        return Err("Windows DPAPI returned an empty output buffer.".to_string());
    }
    // SAFETY: DPAPI returned `cb_data` bytes in a LocalAlloc buffer. Copy before freeing it.
    let bytes = if output.cb_data == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec() }
    };
    if !output.pb_data.is_null() {
        // SAFETY: DPAPI allocated this output with LocalAlloc, as required by its API contract.
        unsafe { local_free(output.pb_data.cast()) };
    }
    Ok(bytes)
}

#[cfg(not(windows))]
fn dpapi_protect(_plain: &[u8]) -> Result<Vec<u8>, String> {
    Err("Diskora Pro entitlement storage requires Windows DPAPI.".to_string())
}

#[cfg(not(windows))]
fn dpapi_unprotect(_encrypted: &[u8]) -> Result<Vec<u8>, String> {
    Err("Diskora Pro entitlement storage requires Windows DPAPI.".to_string())
}

/// What the app may do, as decided by the licensing module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProDecision {
    /// A stored entitlement was just re-verified with Dodo Payments.
    Verified(StoredEntitlement),
    /// Dodo was unreachable, but this entitlement was verified recently enough.
    OfflineGrace(StoredEntitlement),
    /// No purchase is stored on this device.
    Locked,
    /// Dodo answered that the stored key is no longer valid (revoked, refunded, expired, or over
    /// its activation limit). The stored entitlement has been removed.
    Revoked,
}

impl ProDecision {
    pub fn is_active(&self) -> bool {
        matches!(self, ProDecision::Verified(_) | ProDecision::OfflineGrace(_))
    }

    pub fn entitlement(&self) -> Option<&StoredEntitlement> {
        match self {
            ProDecision::Verified(entitlement) | ProDecision::OfflineGrace(entitlement) => {
                Some(entitlement)
            }
            ProDecision::Locked | ProDecision::Revoked => None,
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
    /// Machine-readable state: `active`, `offline`, `not-purchased`, `invalid`, `unverified`
    /// or `not-configured`. Drives the UI copy and badges; the message stays human-readable.
    pub state: String,
    /// Masked key (`ABCD…WXYZ`), when an entitlement is stored.
    pub license_key: Option<String>,
    pub activated_at: Option<i64>,
    pub last_validated_at: Option<i64>,
    /// True when the active state rests on the offline grace period right now.
    pub offline: bool,
    pub message: String,
}

impl ProStatus {
    fn locked(configured: bool, state: &str, message: String) -> Self {
        Self {
            enabled: false,
            configured,
            state: state.to_string(),
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
            if configured {
                ProStatus::locked(configured, "not-purchased", NOT_PURCHASED.to_string())
            } else {
                ProStatus::locked(configured, "not-configured", NOT_CONFIGURED.to_string())
            }
        }
        ProDecision::Revoked => ProStatus::locked(configured, "invalid", REVOKED.to_string()),
        ProDecision::OfflineGrace(entitlement) => ProStatus {
            enabled: true,
            configured,
            state: "offline".to_string(),
            license_key: Some(mask_key(&entitlement.license_key)),
            activated_at: Some(entitlement.activated_at),
            last_validated_at: Some(entitlement.last_validated_at),
            offline: true,
            message: "Diskora Pro is active. Dodo Payments was unreachable, so the last verified purchase is being trusted until the connection returns. Deletion still needs a live check.".to_string(),
        },
        ProDecision::Verified(entitlement) => ProStatus {
            enabled: true,
            configured,
            state: "active".to_string(),
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
        if !api_base_is_allowlisted(&api_base) {
            return Err("The licensing API host is not on Diskora's allowlist.".to_string());
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
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
            // Treat transient service/rate-limit responses as unavailable so a paid customer is not
            // permanently cleared by an outage. Other client errors represent an invalid key.
            if status.is_server_error() || status.as_u16() == 408 || status.as_u16() == 429 {
                return Err(format!("Dodo Payments could not validate the key (HTTP {}).", status.as_u16()));
            }
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
        Some(stored) => {
            let (json, needs_protection) = match unprotect_entitlement(&stored).map_err(AppError::Database)? {
                Some(json) => (json, false),
                None => (stored, true), // Migrate an older plaintext entitlement on first read.
            };
            let entitlement = serde_json::from_str::<StoredEntitlement>(&json)
                .map_err(|error| AppError::Database(format!("The stored entitlement is unreadable: {error}")))?;
            if needs_protection {
                let protected = protect_entitlement(&json).map_err(AppError::Database)?;
                database.save_entitlement(&protected).map_err(AppError::Database)?;
            }
            Ok(Some(entitlement))
        }
        None => Ok(None),
    }
}

/// The startup status: fast, honest, and never more optimistic than the last
/// verified check plus the offline grace window. Publishes the result so the
/// path-disclosure layer follows the same rule.
pub fn cached_status(state: &AppState) -> AppResult<ProStatus> {
    let status = cached_status_inner(state)?;
    crate::disclosure::set_full_paths(status.enabled);
    Ok(status)
}

fn cached_status_inner(state: &AppState) -> AppResult<ProStatus> {
    let stored = stored_entitlement(state)?;
    if !is_configured() {
        return Ok(status_for(&ProDecision::Locked));
    }
    if stored.as_ref().is_some_and(|entitlement| entitlement.product_id != config().product_id) {
        clear_stored(state)?;
        return Ok(status_for(&ProDecision::Locked));
    }
    match stored {
        None => Ok(status_for(&ProDecision::Locked)),
        Some(entitlement) if within_grace(&entitlement, crate::time::now_millis()) => {
            Ok(cached_active_status(entitlement))
        }
        Some(_) => Ok(ProStatus::locked(is_configured(), "unverified", UNVERIFIED.to_string())),
    }
}

/// A stored entitlement that was verified recently. It is trusted at startup without a network
/// call; the Pro page, Restore and every deletion re-check it with Dodo Payments.
fn cached_active_status(entitlement: StoredEntitlement) -> ProStatus {
    let mut status = status_for(&ProDecision::Verified(entitlement));
    status.message = "Diskora Pro is active. This is the last verified state. Restore purchases re-checks it with Dodo Payments, and every deletion is checked live.".to_string();
    status
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
    let result = verified_decision_inner(state).await;
    // Every verification updates the disclosure rule, including failures (fail closed).
    crate::disclosure::set_full_paths(matches!(&result, Ok(decision) if decision.is_active()));
    result
}

async fn verified_decision_inner(state: &AppState) -> AppResult<ProDecision> {
    let Some(entitlement) = stored_entitlement(state)? else {
        return Ok(ProDecision::Locked);
    };
    let config = config();
    if !is_configured() {
        return Ok(ProDecision::Locked);
    }
    if entitlement.product_id != config.product_id {
        clear_stored(state)?;
        return Ok(ProDecision::Locked);
    }
    let client = DodoClient::new(config.api_base).map_err(AppError::Licence)?;
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
            Ok(ProDecision::Revoked)
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

fn require_fresh_verification(decision: &ProDecision) -> AppResult<()> {
    match decision {
        ProDecision::Verified(_) => Ok(()),
        ProDecision::OfflineGrace(_) => Err(AppError::Licence(
            "Deletion requires a fresh Dodo Payments verification. Reconnect and try again; offline grace does not authorize deletion.".to_string(),
        )),
        ProDecision::Locked => Err(AppError::Licence(NOT_PURCHASED.to_string())),
        ProDecision::Revoked => Err(AppError::Licence(REVOKED.to_string())),
    }
}

/// Gate for the non-destructive Pro features (reveal in Explorer, open, full-path export).
/// It reads the verification state that was last published, so it never touches the network.
/// Deletion uses `require_pro` instead, which always re-verifies.
pub fn require_active_pro() -> AppResult<()> {
    if crate::disclosure::full_paths_allowed() {
        Ok(())
    } else {
        Err(AppError::Licence(NOT_PURCHASED.to_string()))
    }
}

/// Deletion requires a successful, fresh Dodo verification; offline grace may show Pro as active
/// but never authorizes a destructive operation.
pub async fn require_pro(state: &AppState) -> AppResult<()> {
    let decision = verified_decision(state).await?;
    require_fresh_verification(&decision)
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
    crate::disclosure::set_full_paths(true);
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
    let json = serde_json::to_string(entitlement)
        .map_err(|error| AppError::Internal(format!("The entitlement could not be serialized: {error}")))?;
    let payload = protect_entitlement(&json).map_err(AppError::Database)?;
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
    fn licensing_hosts_are_https_and_allowlisted() {
        let valid = LicensingConfig {
            api_base: "https://live.dodopayments.com".to_string(),
            product_id: "pdt_example".to_string(),
            checkout_url: "https://checkout.dodopayments.com/buy/example".to_string(),
            environment: "live",
        };
        assert!(config_hosts_are_allowlisted(&valid));

        let mut arbitrary_api = valid.clone();
        arbitrary_api.api_base = "https://attacker.example".to_string();
        assert!(!config_hosts_are_allowlisted(&arbitrary_api));

        let mut deceptive_checkout = valid.clone();
        deceptive_checkout.checkout_url = "https://checkout.dodopayments.com.attacker.example/buy".to_string();
        assert!(!config_hosts_are_allowlisted(&deceptive_checkout));

        let mut credential_url = valid.clone();
        credential_url.checkout_url = "https://checkout.dodopayments.com@attacker.example/buy".to_string();
        assert!(!config_hosts_are_allowlisted(&credential_url));
    }

    #[test]
    fn the_license_api_client_rejects_unallowlisted_hosts_and_paths() {
        assert!(DodoClient::new("https://attacker.example".to_string()).is_err());
        assert!(DodoClient::new("https://live.dodopayments.com/unexpected".to_string()).is_err());
        assert!(DodoClient::new("https://live.dodopayments.com".to_string()).is_ok());
    }

    #[test]
    fn offline_grace_cannot_authorize_deletion() {
        assert!(require_fresh_verification(&ProDecision::Verified(entitlement(NOW))).is_ok());
        let error = require_fresh_verification(&ProDecision::OfflineGrace(entitlement(NOW))).unwrap_err();
        assert!(error.to_string().contains("offline grace does not authorize deletion"));
        assert!(require_fresh_verification(&ProDecision::Locked).is_err());
    }

    #[test]
    fn protected_entitlement_encoding_is_strict() {
        assert_eq!(decode_hex("00aF").unwrap(), vec![0, 0xaf]);
        assert!(decode_hex("0").is_err());
        assert!(decode_hex("zz").is_err());
        assert_eq!(encode_hex(&[0, 0xaf]), "00af");
    }

    #[cfg(windows)]
    #[test]
    fn user_scoped_dpapi_round_trips_entitlement_payloads() {
        let clear = br#"{"licenseKey":"do-not-store-in-plaintext"}"#;
        let protected = dpapi_protect(clear).unwrap();
        assert_ne!(protected, clear);
        assert_eq!(dpapi_unprotect(&protected).unwrap(), clear);
    }

    #[test]
    fn device_names_are_bounded_and_never_empty_of_context() {
        let name = device_name();
        assert!(!name.is_empty());
        assert!(name.len() <= 64);
    }
}

#[cfg(test)]
mod access_tests {
    use super::*;
    use crate::disclosure::{full_paths_allowed, set_full_paths};

    const NOW: i64 = 10_000_000_000;

    fn stored(last_validated_at: i64) -> StoredEntitlement {
        StoredEntitlement {
            license_key: "ABCD-1234-EFGH-5678".to_string(),
            instance_id: Some("lki_123".to_string()),
            product_id: "pdt_test".to_string(),
            device_name: "Diskora on TEST-PC".to_string(),
            activated_at: 1_000,
            last_validated_at,
        }
    }

    #[test]
    fn a_revoked_licence_is_reported_as_invalid_and_never_active() {
        let decision = ProDecision::Revoked;
        assert!(!decision.is_active());
        assert!(decision.entitlement().is_none());
        let status = status_for(&decision);
        assert!(!status.enabled);
        assert_eq!(status.state, "invalid");
        assert!(status.message.contains("no longer valid"));
        assert!(require_fresh_verification(&decision).is_err());
    }

    #[test]
    fn invalid_or_expired_licences_fall_back_to_free() {
        // A key Dodo rejects never unlocks, even inside the grace window.
        assert!(!decide(Some(stored(NOW)), Some(false), NOW).is_active());
        // A stored key that has not been re-verified within the grace window is not active.
        let expired = decide(Some(stored(NOW - OFFLINE_GRACE_MS - 1)), None, NOW);
        assert!(!expired.is_active());
        assert!(matches!(status_for(&expired).state.as_str(), "not-purchased" | "not-configured"));
    }

    #[test]
    fn status_states_are_distinct_and_machine_readable() {
        assert_eq!(status_for(&ProDecision::Verified(stored(NOW))).state, "active");
        assert_eq!(status_for(&ProDecision::OfflineGrace(stored(NOW))).state, "offline");
        assert_eq!(status_for(&ProDecision::Locked).state, if is_configured() { "not-purchased" } else { "not-configured" });
    }

    /// One test owns the process-wide access flag, so parallel tests cannot race on it.
    #[test]
    fn the_free_and_pro_gates_follow_the_verified_state() {
        // Free: no licence, no reveal, no delete.
        set_full_paths(false);
        assert!(!full_paths_allowed());
        assert!(require_active_pro().is_err(), "reveal/export must be refused for free");
        assert!(require_fresh_verification(&ProDecision::Locked).is_err(), "delete must be refused");
        assert!(require_fresh_verification(&ProDecision::Revoked).is_err());
        assert!(require_fresh_verification(&ProDecision::OfflineGrace(stored(NOW))).is_err());

        // Pro: a fresh verification unlocks both reveal/export and delete.
        let verified = ProDecision::Verified(stored(NOW));
        set_full_paths(verified.is_active());
        assert!(full_paths_allowed());
        assert!(require_active_pro().is_ok());
        assert!(require_fresh_verification(&verified).is_ok());

        // Back to free after the licence is revoked.
        set_full_paths(ProDecision::Revoked.is_active());
        assert!(require_active_pro().is_err());
        set_full_paths(false);
    }
}
