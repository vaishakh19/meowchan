//! The signed licence payload.
//!
//! A licence key is `<base64url payload>.<base64url signature>`. The payload is JSON describing
//! what was bought and who it was issued to; the signature is Ed25519 over the payload bytes,
//! produced by the backend's private key and verified here against the public key named by
//! `key_id`.
//!
//! The payload is **versioned**. `schema_version` is checked before anything else, so a future
//! payload shape cannot be replayed against an older build that would misinterpret it.
//!
//! ## Permanent ownership vs. an offline lease
//!
//! `permanent: true` means the entitlement never expires — the customer owns the tier.
//! `lease_until` is a separate, deliberately short window during which the app will keep working
//! without contacting the server. When the lease lapses the app re-checks; if it cannot, it falls
//! back to free **and says so**, rather than silently degrading.
//!
//! `device_binding` is a *binding*, not a secret. It lets the backend enforce the device limit, but
//! a determined user can copy it — which is exactly why `docs/licensing.md` states plainly that
//! local DRM does not make copying impossible.

use std::path::{Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::config::product::{decode_key, CommercialConfig};

/// Current payload schema version. Bump when the shape changes incompatibly.
pub const SCHEMA_VERSION: u32 = 1;

/// The signed contents of a licence key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LicensePayload {
    /// Schema version. Must equal what this build understands.
    pub schema_version: u32,
    /// Which signing key produced this payload, so rotation is unambiguous.
    pub key_id: String,
    /// Stable licence identifier, never an email address.
    pub license_id: String,
    /// Product this licence is for. Must match the configured product.
    pub product_id: String,
    /// Tier identifier, e.g. `pro-lifetime`.
    pub tier_id: String,
    /// Unix milliseconds.
    pub issued_at: i64,
    /// `true` for a one-time purchase: the entitlement itself never expires.
    pub permanent: bool,
    ///
    /// Unix milliseconds after which the app must re-validate with the server. `None` for a
    /// perpetual lease (only sensible with `permanent`). This is **not** the entitlement expiry.
    pub lease_until: Option<i64>,
    /// Device binding this licence was issued for, when it was activated on one device.
    pub device_binding: Option<String>,
    /// How many devices this licence may be active on at once.
    pub device_limit: u32,
    /// Free-form, backend-controlled metadata. Never contains personal data.
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, String>,
}

impl LicensePayload {
    /// A payload for a permanent, one-time-purchase licence.
    pub fn permanent(
        license_id: impl Into<String>,
        product_id: impl Into<String>,
        tier_id: impl Into<String>,
        key_id: impl Into<String>,
        issued_at: i64,
        device_limit: u32,
    ) -> Self {
        LicensePayload {
            schema_version: SCHEMA_VERSION,
            key_id: key_id.into(),
            license_id: license_id.into(),
            product_id: product_id.into(),
            tier_id: tier_id.into(),
            issued_at,
            permanent: true,
            lease_until: None,
            device_binding: None,
            device_limit,
            metadata: Default::default(),
        }
    }

    /// True when the offline lease has lapsed and the app must re-validate.
    ///
    /// A perpetual lease (`None`) never lapses. A permanent entitlement with a lapsed lease is
    /// still *owned*; it just cannot be proven offline right now.
    pub fn lease_lapsed(&self, now: i64) -> bool {
        match self.lease_until {
            Some(until) => now >= until,
            None => false,
        }
    }
}

/// A licence that has been parsed but not yet trusted.
#[derive(Debug, Clone)]
pub struct ParsedLicense {
    pub payload: LicensePayload,
    pub signature: Vec<u8>,
    /// The exact key string, for masking and display.
    pub key: String,
}

/// Why a licence key could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenseError {
    Empty,
    Malformed(&'static str),
    UnknownSchema { found: u32, supported: u32 },
    UnknownKey { key_id: String },
    BadSignature,
    WrongProduct { expected: String, found: String },
    UnknownTier { tier_id: String },
    NotYetValid { issued_at: i64, now: i64 },
    LeaseLapsed { until: i64, now: i64 },
    WrongDevice { expected: String, found: Option<String> },
    DeviceLimitExceeded { limit: u32 },
}

impl LicenseError {
    /// A message safe to show a customer. No internals, no key material.
    pub fn message(&self) -> String {
        match self {
            LicenseError::Empty => "No licence key was provided.".to_string(),
            LicenseError::Malformed(reason) => format!("The licence key is malformed ({reason})."),
            LicenseError::UnknownSchema { found, supported } => format!(
                "This licence was issued for a newer version of Diskwise (schema {found}, this build understands {supported}). Update Diskwise to use it."
            ),
            LicenseError::UnknownKey { .. } => {
                "This licence was signed with a key this version does not trust. Update Diskwise, or contact support if you already have the latest version.".to_string()
            }
            LicenseError::BadSignature => {
                "This licence key failed signature verification and was not accepted.".to_string()
            }
            LicenseError::WrongProduct { .. } => {
                "This licence is for a different product.".to_string()
            }
            LicenseError::UnknownTier { .. } => {
                "This licence refers to a plan this version does not recognise.".to_string()
            }
            LicenseError::NotYetValid { .. } => {
                "This licence is not valid yet. Check that your computer's clock is correct.".to_string()
            }
            LicenseError::LeaseLapsed { .. } => {
                "This licence needs to be re-verified with the licence server. Connect to the internet and re-activate; your purchase is not affected.".to_string()
            }
            LicenseError::WrongDevice { .. } => {
                "This licence is activated on a different computer. Deactivate it there first, or contact support to move it.".to_string()
            }
            LicenseError::DeviceLimitExceeded { limit } => format!(
                "This licence is already active on {limit} computers. Deactivate one to continue."
            ),
        }
    }
}

/// Splits and decodes a licence key without verifying anything.
pub fn parse(key: &str) -> Result<ParsedLicense, LicenseError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(LicenseError::Empty);
    }
    // Tolerate surrounding whitespace and a trailing newline from a paste.
    let trimmed = trimmed.trim_matches(|c: char| c.is_whitespace() || c == '\r' || c == '\n');

    let mut parts = trimmed.split('.');
    let payload_b64 = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or(LicenseError::Malformed("missing payload"))?;
    let signature_b64 = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or(LicenseError::Malformed("missing signature"))?;
    if parts.next().is_some() {
        return Err(LicenseError::Malformed("extra segments"));
    }

    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|_| LicenseError::Malformed("payload is not base64url"))?;
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|_| LicenseError::Malformed("signature is not base64url"))?;
    if signature.len() != 64 {
        return Err(LicenseError::Malformed("signature is not 64 bytes"));
    }

    let payload: LicensePayload =
        serde_json::from_slice(&payload_bytes).map_err(|_| LicenseError::Malformed("payload is not JSON"))?;

    Ok(ParsedLicense {
        payload,
        signature,
        key: trimmed.to_string(),
    })
}

/// Verifies a parsed licence against the configuration.
///
/// Order matters: shape, then schema, then key identity, then signature, then policy. The signature
/// is checked against the key the payload *names*, so a payload cannot pick a weaker key.
pub fn verify(
    parsed: &ParsedLicense,
    config: &CommercialConfig,
    now: i64,
    device_binding: Option<&str>,
) -> Result<LicensePayload, LicenseError> {
    let payload = &parsed.payload;

    if payload.schema_version != config.license_schema_version {
        return Err(LicenseError::UnknownSchema {
            found: payload.schema_version,
            supported: config.license_schema_version,
        });
    }

    let trusted = config
        .key(&payload.key_id)
        .ok_or_else(|| LicenseError::UnknownKey {
            key_id: payload.key_id.clone(),
        })?;

    if !signature_valid(&parsed.payload_bytes(), &parsed.signature, &trusted.public_key_base64) {
        return Err(LicenseError::BadSignature);
    }

    if payload.product_id != config.product_id {
        return Err(LicenseError::WrongProduct {
            expected: config.product_id.clone(),
            found: payload.product_id.clone(),
        });
    }

    let tier = config
        .tier(&payload.tier_id)
        .ok_or_else(|| LicenseError::UnknownTier {
            tier_id: payload.tier_id.clone(),
        })?;
    if !tier.available {
        return Err(LicenseError::UnknownTier {
            tier_id: payload.tier_id.clone(),
        });
    }

    // Allow a small clock skew before declaring a licence "not yet valid".
    if payload.issued_at > now + 5 * 60 * 1000 {
        return Err(LicenseError::NotYetValid {
            issued_at: payload.issued_at,
            now,
        });
    }

    if payload.device_limit == 0 || payload.device_limit > 100 {
        return Err(LicenseError::DeviceLimitExceeded {
            limit: payload.device_limit,
        });
    }

    if let Some(expected) = device_binding {
        match &payload.device_binding {
            Some(bound) if bound == expected => {}
            Some(bound) => {
                return Err(LicenseError::WrongDevice {
                    expected: expected.to_string(),
                    found: Some(bound.clone()),
                })
            }
            None => {
                return Err(LicenseError::WrongDevice {
                    expected: expected.to_string(),
                    found: None,
                })
            }
        }
    }

    if payload.lease_lapsed(now) {
        return Err(LicenseError::LeaseLapsed {
            until: payload.lease_until.unwrap_or_default(),
            now,
        });
    }

    Ok(payload.clone())
}

/// Ed25519 verification against a base64 public key.
fn signature_valid(payload: &[u8], signature: &[u8], public_key_base64: &str) -> bool {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let Some(key_bytes) = decode_key(public_key_base64) else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&key_bytes) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(signature) else {
        return false;
    };
    verifying_key.verify(payload, &signature).is_ok()
}

/// The exact bytes the signature covers. Kept here so signing (backend) and verifying (app) can
/// never disagree about what is signed.
pub fn payload_bytes(payload: &LicensePayload) -> Vec<u8> {
    serde_json::to_vec(payload).unwrap_or_default()
}

impl ParsedLicense {
    /// The canonical bytes that were signed.
    pub fn payload_bytes(&self) -> Vec<u8> {
        payload_bytes(&self.payload)
    }
}

/// Encodes a payload and signature into a licence key. Used by the backend and by the tests.
pub fn encode(payload: &LicensePayload, signature: &[u8]) -> String {
    let encoded_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_bytes(payload));
    let encoded_signature = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature);
    format!("{encoded_payload}.{encoded_signature}")
}

/// Masks all but the last four characters of a key.
pub fn mask_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.chars().count() <= 4 {
        return "****".to_string();
    }
    let visible: String = trimmed.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    let hidden = "*".repeat(trimmed.chars().count().saturating_sub(4));
    format!("{hidden}{visible}")
}

/* ------------------------------------------------------------------ *\
 * Device binding
\* ------------------------------------------------------------------ */

/// A stable, non-secret device identifier.
///
/// Derived from the machine name, the OS install date and the volume the app is installed on. It is
/// deliberately **not** a secret: it exists so the backend can count devices, not to authenticate
/// one. See `docs/licensing.md` ("Device binding is not a secret").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceBinding {
    pub value: String,
}

impl DeviceBinding {
    /// Computes the binding for this machine.
    pub fn current(data_dir: &Path) -> DeviceBinding {
        let mut parts: Vec<String> = Vec::new();

        // Machine name, lower-cased and truncated.
        let host = hostname(data_dir);
        parts.push(host);

        // OS install date, when available. Stable across reboots, changes on reinstall.
        #[cfg(windows)]
        {
            if let Some(install) = os_install_millis() {
                parts.push(format!("os-{install}"));
            }
        }

        // The volume the data directory lives on. Stable, and changes when the app moves drive.
        let volume = data_dir
            .components()
            .next()
            .map(|component| component.as_os_str().to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        parts.push(format!("vol-{volume}"));

        let joined = parts.join("|");
        DeviceBinding {
            value: format!("dev-{}", short_hash(&joined)),
        }
    }

    /// True when this binding matches the one in a payload.
    pub fn matches(&self, payload: &LicensePayload) -> bool {
        match &payload.device_binding {
            Some(bound) => bound == &self.value,
            None => false,
        }
    }
}

fn hostname(data_dir: &Path) -> String {
    // Prefer the real hostname; fall back to something stable and non-identifying.
    #[cfg(windows)]
    let _ = data_dir;
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default();
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect();
    if cleaned.is_empty() {
        "unknown-host".to_string()
    } else {
        cleaned.to_lowercase()
    }
}

#[cfg(windows)]
fn os_install_millis() -> Option<i64> {
    // The registry key is the documented source; read it without pulling in a registry crate.
    let output = std::process::Command::new("reg")
        .args([
            "query",
            "HKLM\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
            "/v",
            "InstallDate",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // The value line looks like: `    InstallDate    REG_DWORD    0x63f1a2b3`
    for token in text.split_whitespace() {
        if let Some(hex) = token.strip_prefix("0x") {
            if let Ok(seconds) = u64::from_str_radix(hex, 16) {
                return Some(seconds as i64 * 1000);
            }
        }
    }
    None
}

/// A short, stable, non-reversible digest. FNV-1a, matching `stablePathId` elsewhere.
fn short_hash(input: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The directory a device's local activation record lives in.
pub fn activation_record_path(data_dir: &Path) -> PathBuf {
    data_dir.join("activation.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::product::{CommercialConfig, Environment, ProductTier, TrustedKey};
    use ed25519_dalek::{Signer, SigningKey};

    /// A deterministic test keypair. Never used outside tests.
    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn test_config(key_id: &str, public: &str) -> CommercialConfig {
        CommercialConfig {
            product_id: "diskwise-pro".to_string(),
            product_name: "Diskwise Pro".to_string(),
            license_schema_version: 1,
            environment: Environment::Test,
            tiers: vec![ProductTier {
                id: "pro-lifetime".to_string(),
                name: "Diskwise Pro".to_string(),
                price_minor_units: 149_900,
                currency: "INR".to_string(),
                permanent: true,
                update_eligibility_months: None,
                available: true,
            }],
            device_limit: 3,
            offline_lease_days: 30,
            free_duplicate_group_limit: 25,
            trusted_keys: vec![TrustedKey {
                key_id: key_id.to_string(),
                public_key_base64: public.to_string(),
                active: true,
                note: "test".to_string(),
            }],
        }
    }

    fn public_base64(signing: &SigningKey) -> String {
        base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes())
    }

    fn sign(signing: &SigningKey, payload: &LicensePayload) -> Vec<u8> {
        signing.sign(&payload_bytes(payload)).to_bytes().to_vec()
    }

    fn base_payload() -> LicensePayload {
        LicensePayload::permanent(
            "lic-123",
            "diskwise-pro",
            "pro-lifetime",
            "k-test",
            1_700_000_000_000,
            3,
        )
    }

    const NOW: i64 = 1_700_000_100_000;

    /* ------------------------------- happy path ------------------------------ */

    #[test]
    fn a_correctly_signed_permanent_licence_verifies() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let payload = base_payload();
        let key = encode(&payload, &sign(&signing, &payload));

        let parsed = parse(&key).expect("parses");
        let verified = verify(&parsed, &config, NOW, None).expect("verifies");
        assert_eq!(verified.license_id, "lic-123");
        assert!(verified.permanent);
        assert_eq!(verified.tier_id, "pro-lifetime");
    }

    #[test]
    fn a_permanent_licence_never_expires() {
        let payload = base_payload();
        assert!(payload.permanent);
        assert_eq!(payload.lease_until, None);
        assert!(!payload.lease_lapsed(NOW + 100 * 365 * 86_400_000));
    }

    #[test]
    fn a_lease_expires_but_the_entitlement_does_not() {
        let mut payload = base_payload();
        payload.lease_until = Some(NOW - 1);
        assert!(payload.lease_lapsed(NOW));
        // Ownership is untouched.
        assert!(payload.permanent);
    }

    /* ------------------------------ malformed input ------------------------------ */

    #[test]
    fn an_empty_key_is_rejected() {
        assert_eq!(parse(""), Err(LicenseError::Empty));
        assert_eq!(parse("   "), Err(LicenseError::Empty));
    }

    #[test]
    fn a_key_with_no_signature_is_rejected() {
        assert!(matches!(parse("abc"), Err(LicenseError::Malformed(_))));
    }

    #[test]
    fn a_key_with_extra_segments_is_rejected() {
        assert!(matches!(parse("a.b.c"), Err(LicenseError::Malformed(_))));
    }

    #[test]
    fn a_non_base64_payload_is_rejected() {
        assert!(matches!(
            parse("!!!.???"),
            Err(LicenseError::Malformed(_))
        ));
    }

    #[test]
    fn a_short_signature_is_rejected() {
        let payload = base_payload();
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_bytes(&payload));
        let short = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0u8; 10]);
        assert!(matches!(
            parse(&format!("{encoded}.{short}")),
            Err(LicenseError::Malformed(_))
        ));
    }

    #[test]
    fn a_payload_that_is_not_json_is_rejected() {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"not json");
        let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0u8; 64]);
        assert!(matches!(
            parse(&format!("{encoded}.{signature}")),
            Err(LicenseError::Malformed(_))
        ));
    }

    #[test]
    fn surrounding_whitespace_is_tolerated() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let payload = base_payload();
        let key = encode(&payload, &sign(&signing, &payload));
        let padded = format!("  {key}\n");
        let parsed = parse(&padded).expect("parses");
        assert!(verify(&parsed, &config, NOW, None).is_ok());
    }

    /* -------------------------------- signature -------------------------------- */

    #[test]
    fn a_forged_signature_is_rejected() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let payload = base_payload();
        // Signed by a different key.
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let key = encode(&payload, &sign(&other, &payload));
        let parsed = parse(&key).expect("parses");
        assert_eq!(verify(&parsed, &config, NOW, None), Err(LicenseError::BadSignature));
    }

    #[test]
    fn a_garbage_signature_is_rejected() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let payload = base_payload();
        let key = encode(&payload, &[0u8; 64]);
        let parsed = parse(&key).expect("parses");
        assert_eq!(verify(&parsed, &config, NOW, None), Err(LicenseError::BadSignature));
    }

    #[test]
    fn a_modified_payload_is_rejected() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let payload = base_payload();
        let signature = sign(&signing, &payload);

        // Tamper with the payload after signing.
        let mut tampered = payload.clone();
        tampered.device_limit = 100;
        let key = encode(&tampered, &signature);

        let parsed = parse(&key).expect("parses");
        assert_eq!(verify(&parsed, &config, NOW, None), Err(LicenseError::BadSignature));
    }

    #[test]
    fn a_payload_re_signed_with_a_different_key_id_is_rejected() {
        // The payload names a key; verification must use *that* key, not the newest one.
        let trusted = test_signing_key();
        let attacker = SigningKey::from_bytes(&[11u8; 32]);
        let config = test_config("k-test", &public_base64(&trusted));

        let mut payload = base_payload();
        payload.key_id = "k-attacker".to_string();
        let key = encode(&payload, &sign(&attacker, &payload));
        let parsed = parse(&key).expect("parses");
        assert_eq!(
            verify(&parsed, &config, NOW, None),
            Err(LicenseError::UnknownKey {
                key_id: "k-attacker".to_string()
            })
        );
    }

    /* ------------------------------- schema version ------------------------------- */

    #[test]
    fn an_unknown_schema_version_is_rejected() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let mut payload = base_payload();
        payload.schema_version = 99;
        let key = encode(&payload, &sign(&signing, &payload));
        let parsed = parse(&key).expect("parses");
        assert_eq!(
            verify(&parsed, &config, NOW, None),
            Err(LicenseError::UnknownSchema {
                found: 99,
                supported: 1
            })
        );
    }

    /* --------------------------------- product --------------------------------- */

    #[test]
    fn a_licence_for_another_product_is_rejected() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let mut payload = base_payload();
        payload.product_id = "some-other-app".to_string();
        let key = encode(&payload, &sign(&signing, &payload));
        let parsed = parse(&key).expect("parses");
        assert!(matches!(
            verify(&parsed, &config, NOW, None),
            Err(LicenseError::WrongProduct { .. })
        ));
    }

    #[test]
    fn a_licence_for_an_unknown_tier_is_rejected() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let mut payload = base_payload();
        payload.tier_id = "enterprise".to_string();
        let key = encode(&payload, &sign(&signing, &payload));
        let parsed = parse(&key).expect("parses");
        assert!(matches!(
            verify(&parsed, &config, NOW, None),
            Err(LicenseError::UnknownTier { .. })
        ));
    }

    /* -------------------------------- device binding -------------------------------- */

    #[test]
    fn a_licence_bound_to_another_device_is_rejected() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let mut payload = base_payload();
        payload.device_binding = Some("dev-aaa".to_string());
        let key = encode(&payload, &sign(&signing, &payload));
        let parsed = parse(&key).expect("parses");
        assert!(matches!(
            verify(&parsed, &config, NOW, Some("dev-bbb")),
            Err(LicenseError::WrongDevice { .. })
        ));
    }

    #[test]
    fn a_licence_bound_to_this_device_is_accepted() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let mut payload = base_payload();
        payload.device_binding = Some("dev-aaa".to_string());
        let key = encode(&payload, &sign(&signing, &payload));
        let parsed = parse(&key).expect("parses");
        assert!(verify(&parsed, &config, NOW, Some("dev-aaa")).is_ok());
    }

    #[test]
    fn an_unbound_licence_cannot_be_used_on_a_bound_device() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let payload = base_payload();
        assert_eq!(payload.device_binding, None);
        let key = encode(&payload, &sign(&signing, &payload));
        let parsed = parse(&key).expect("parses");
        assert!(matches!(
            verify(&parsed, &config, NOW, Some("dev-aaa")),
            Err(LicenseError::WrongDevice { .. })
        ));
    }

    #[test]
    fn an_absurd_device_limit_in_a_payload_is_rejected() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let mut payload = base_payload();
        payload.device_limit = 500;
        let key = encode(&payload, &sign(&signing, &payload));
        let parsed = parse(&key).expect("parses");
        assert!(matches!(
            verify(&parsed, &config, NOW, None),
            Err(LicenseError::DeviceLimitExceeded { .. })
        ));
    }

    /* --------------------------------- lease --------------------------------- */

    #[test]
    fn a_lapsed_lease_is_rejected_even_though_the_licence_is_permanent() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let mut payload = base_payload();
        payload.lease_until = Some(NOW - 1);
        let key = encode(&payload, &sign(&signing, &payload));
        let parsed = parse(&key).expect("parses");
        assert_eq!(
            verify(&parsed, &config, NOW, None),
            Err(LicenseError::LeaseLapsed {
                until: NOW - 1,
                now: NOW
            })
        );
        // And the message tells the customer their purchase is safe.
        assert!(LicenseError::LeaseLapsed { until: 0, now: 0 }
            .message()
            .contains("purchase is not affected"));
    }

    #[test]
    fn a_future_issued_at_is_rejected() {
        let signing = test_signing_key();
        let config = test_config("k-test", &public_base64(&signing));
        let mut payload = base_payload();
        payload.issued_at = NOW + 10 * 60 * 1000;
        let key = encode(&payload, &sign(&signing, &payload));
        let parsed = parse(&key).expect("parses");
        assert!(matches!(
            verify(&parsed, &config, NOW, None),
            Err(LicenseError::NotYetValid { .. })
        ));
    }

    /* --------------------------------- masking --------------------------------- */

    #[test]
    fn masking_reveals_only_the_last_four_characters() {
        assert_eq!(mask_key("ABCD-1234-EFGH-5678"), "****************5678");
        assert_eq!(mask_key("abcd"), "****");
        assert_eq!(mask_key(""), "****");
    }

    /* ------------------------------ key rotation ------------------------------ */

    #[test]
    fn an_old_key_still_verifies_after_a_rotation() {
        let old = SigningKey::from_bytes(&[1u8; 32]);
        let new = SigningKey::from_bytes(&[2u8; 32]);
        let mut config = test_config("k-new", &public_base64(&new));
        // The old key is kept so already-issued licences keep working.
        config.trusted_keys.push(TrustedKey {
            key_id: "k-old".to_string(),
            public_key_base64: public_base64(&old),
            active: true,
            note: "rotated out, kept for verification".to_string(),
        });

        let mut payload = base_payload();
        payload.key_id = "k-old".to_string();
        let key = encode(&payload, &sign(&old, &payload));
        let parsed = parse(&key).expect("parses");
        assert!(verify(&parsed, &config, NOW, None).is_ok());
        assert_eq!(config.primary_key().unwrap().key_id, "k-new");
    }

    #[test]
    fn a_deactivated_key_stops_verifying() {
        let signing = test_signing_key();
        let mut config = test_config("k-test", &public_base64(&signing));
        for key in config.trusted_keys.iter_mut() {
            key.active = false;
        }
        let payload = base_payload();
        let key = encode(&payload, &sign(&signing, &payload));
        let parsed = parse(&key).expect("parses");
        assert!(matches!(
            verify(&parsed, &config, NOW, None),
            Err(LicenseError::UnknownKey { .. })
        ));
    }

    /* ------------------------------- device binding ------------------------------- */

    #[test]
    fn a_device_binding_is_stable_and_non_secret() {
        let dir = std::env::temp_dir().join("diskwise-binding-test");
        let first = DeviceBinding::current(&dir);
        let second = DeviceBinding::current(&dir);
        assert_eq!(first, second);
        assert!(first.value.starts_with("dev-"));
        assert_eq!(first.value.len(), 4 + 16);
        // It is a hash, not the hostname.
        assert!(!first.value.contains("hostname"));
    }

    #[test]
    fn a_device_binding_matches_only_its_own_payload() {
        let mut payload = base_payload();
        payload.device_binding = Some("dev-abc".to_string());
        let binding = DeviceBinding {
            value: "dev-abc".to_string(),
        };
        assert!(binding.matches(&payload));
        let other = DeviceBinding {
            value: "dev-xyz".to_string(),
        };
        assert!(!other.matches(&payload));
    }

    #[test]
    fn the_activation_record_path_is_inside_the_data_directory() {
        let dir = std::path::PathBuf::from("C:\\Users\\me\\AppData\\Roaming\\com.diskwise.app");
        assert!(activation_record_path(&dir)
            .starts_with(&dir));
    }

    /* --------------------------------- messages --------------------------------- */

    #[test]
    fn every_error_message_is_customer_facing_and_non_empty() {
        let errors = [
            LicenseError::Empty,
            LicenseError::Malformed("x"),
            LicenseError::UnknownSchema {
                found: 2,
                supported: 1,
            },
            LicenseError::UnknownKey {
                key_id: "k".to_string(),
            },
            LicenseError::BadSignature,
            LicenseError::WrongProduct {
                expected: "a".to_string(),
                found: "b".to_string(),
            },
            LicenseError::UnknownTier {
                tier_id: "t".to_string(),
            },
            LicenseError::NotYetValid {
                issued_at: 1,
                now: 0,
            },
            LicenseError::LeaseLapsed { until: 1, now: 2 },
            LicenseError::WrongDevice {
                expected: "a".to_string(),
                found: None,
            },
            LicenseError::DeviceLimitExceeded { limit: 3 },
        ];
        for error in errors {
            let message = error.message();
            assert!(!message.is_empty(), "{error:?} has no message");
            // No internals leak into a customer-facing string.
            assert!(!message.contains("0x"), "{error:?} leaks hex");
            assert!(!message.contains("panicked"), "{error:?} leaks internals");
        }
    }

    #[test]
    fn a_wrong_key_error_does_not_leak_the_key_id() {
        // The customer does not need to know which key id failed.
        let message = LicenseError::UnknownKey {
            key_id: "k-secret-rotation".to_string(),
        }
        .message();
        assert!(!message.contains("k-secret-rotation"));
    }
}
