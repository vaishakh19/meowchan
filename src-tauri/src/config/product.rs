//! The product and entitlement configuration.
//!
//! This is the single source of truth for commercial values. It is embedded in the binary so a
//! tampered settings file cannot raise a device limit or extend an entitlement, and it is validated
//! on load so a typo cannot silently grant access.
//!
//! ## One-time purchase, not a subscription
//!
//! Diskwise is sold as a **one-time purchase**. `tiers[*].permanent = true` means the entitlement
//! itself never expires: a customer who paid once keeps the paid features for the lifetime of the
//! product. What *is* time-limited is the **offline validation lease**
//! (`offline_lease_days`) — a re-check cadence that lets the backend revoke a refunded or
//! chargebacked licence without requiring an internet connection to keep working.
//!
//! Those two things are deliberately separate fields, and `docs/licensing.md` explains the
//! distinction in customer-facing language. Nothing in this file may describe a permanent
//! entitlement as expiring, or a lease as ownership.

use serde::{Deserialize, Serialize};

/// Which payment environment this build talks to.
///
/// A build is **either** test or live; there is no runtime switch, so a test build cannot mint a
/// production entitlement and a live build cannot be pointed at the sandbox by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Test,
    Live,
}

impl Environment {
    pub fn as_str(&self) -> &'static str {
        match self {
            Environment::Test => "test",
            Environment::Live => "live",
        }
    }

    /// True when this build must never grant a real entitlement.
    pub fn is_test(&self) -> bool {
        matches!(self, Environment::Test)
    }
}

/// One purchasable tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductTier {
    /// Stable identifier used inside signed licence payloads. Never localised.
    pub id: String,
    /// Customer-facing name.
    pub name: String,
    /// Price in the smallest currency unit (paise for INR, cents for USD).
    pub price_minor_units: u64,
    /// ISO 4217 currency code.
    pub currency: String,
    /// `true` for a one-time purchase: the entitlement itself never expires.
    pub permanent: bool,
    /// How long the customer is entitled to updates for this tier, in months.
    /// `None` means updates are included for the life of the product.
    pub update_eligibility_months: Option<u32>,
    /// Whether this tier is offered in this build.
    pub available: bool,
}

/// A public verification key and the identifier it is known by.
///
/// `key_id` appears in every signed payload so a rotated key can be told apart from a revoked one
/// without guessing, and so old licences keep verifying after a rotation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedKey {
    /// Short, stable identifier, e.g. `k2026-01`.
    pub key_id: String,
    /// Base64-encoded Ed25519 public key. Never a private key.
    pub public_key_base64: String,
    /// Keys with `active = false` are kept only so already-issued licences keep verifying.
    pub active: bool,
    /// Human note about when and why this key was introduced.
    pub note: String,
}

/// The complete commercial configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommercialConfig {
    /// Identifier of the product this build sells. Must match the backend's `products.id`.
    pub product_id: String,
    pub product_name: String,
    /// Version of the signed payload schema this build understands.
    pub license_schema_version: u32,
    /// Payment environment for this build.
    pub environment: Environment,
    /// The tiers this build knows about. The first entry is the default.
    pub tiers: Vec<ProductTier>,
    /// How many devices one licence may be active on at once.
    pub device_limit: u32,
    /// How long a previously-verified licence keeps working with no re-validation.
    ///
    /// This is a **lease**, not ownership: it bounds the damage a refunded licence can do while
    /// still letting a customer work offline for weeks.
    pub offline_lease_days: i64,
    /// Free-tier duplicate group limit. `0` means unlimited.
    pub free_duplicate_group_limit: u32,
    /// Verification keys, newest first. At least one must be active.
    pub trusted_keys: Vec<TrustedKey>,
}

/// The configuration shipped in this build.
///
/// **This is the only place these numbers exist.** `docs/licensing.md` records how to change them,
/// and `config::product::tests` asserts the invariants that make a change safe.
pub const DEFAULT_CONFIG: CommercialConfig = CommercialConfig {
    product_id: "diskwise-pro",
    product_name: "Diskwise Pro",
    license_schema_version: 1,
    // Test mode is the default so a build produced by `npm run app:build` cannot mint a real
    // entitlement. Flipping this is a deliberate, reviewed release action — see
    // `docs/licensing.md` ("Switching to live mode").
    environment: Environment::Test,
    tiers: &[ProductTier {
        id: "pro-lifetime",
        name: "Diskwise Pro (lifetime)",
        // ₹1,499 = 149900 paise. A proposed experiment, not a verified market price; the value is
        // configurable here and published from the backend, never hardcoded in the UI.
        price_minor_units: 149_900,
        currency: "INR",
        permanent: true,
        update_eligibility_months: None,
        available: true,
    }],
    device_limit: 3,
    offline_lease_days: 30,
    free_duplicate_group_limit: 25,
    trusted_keys: &[TrustedKey {
        // A placeholder public key. Replace before shipping: `docs/licensing.md` documents the
        // generation command. It is a *public* key, so it is safe to commit.
        key_id: "k2026-01",
        public_key_base64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        active: true,
        note: "Placeholder verification key. Replace with the key generated by the release process.",
    }],
};

impl CommercialConfig {
    /// Validates the configuration. Returns the first problem found.
    ///
    /// Called at startup and in the tests, so a malformed configuration fails loudly instead of
    /// quietly granting or denying access.
    pub fn validate(&self) -> Result<(), String> {
        if self.product_id.trim().is_empty() {
            return Err("product_id must not be empty".to_string());
        }
        if self.product_name.trim().is_empty() {
            return Err("product_name must not be empty".to_string());
        }
        if self.license_schema_version == 0 {
            return Err("license_schema_version must be at least 1".to_string());
        }
        if self.tiers.is_empty() {
            return Err("at least one tier must be configured".to_string());
        }
        if self.device_limit == 0 {
            return Err("device_limit must be at least 1".to_string());
        }
        if self.device_limit > 100 {
            return Err("device_limit above 100 is almost certainly a mistake".to_string());
        }
        if self.offline_lease_days < 1 {
            return Err("offline_lease_days must be at least 1".to_string());
        }
        if self.offline_lease_days > 365 {
            return Err("offline_lease_days above 365 defeats the purpose of a lease".to_string());
        }
        if self.trusted_keys.is_empty() {
            return Err("at least one trusted verification key is required".to_string());
        }
        if !self.trusted_keys.iter().any(|key| key.active) {
            return Err("at least one trusted verification key must be active".to_string());
        }

        for tier in &self.tiers {
            if tier.id.trim().is_empty() {
                return Err("every tier needs an id".to_string());
            }
            if tier.name.trim().is_empty() {
                return Err(format!("tier {} needs a name", tier.id));
            }
            if tier.currency.len() != 3 || !tier.currency.chars().all(|c| c.is_ascii_uppercase()) {
                return Err(format!(
                    "tier {} currency must be a 3-letter ISO 4217 code, got {}",
                    tier.id, tier.currency
                ));
            }
            // A free tier is legitimately priced at zero; anything else must cost something.
            if tier.price_minor_units == 0 && tier.permanent && tier.available {
                return Err(format!(
                    "tier {} is a paid lifetime tier but costs nothing",
                    tier.id
                ));
            }
        }

        // Tier ids must be unique, or a signed payload could name two different tiers.
        let mut seen = std::collections::HashSet::new();
        for tier in &self.tiers {
            if !seen.insert(tier.id.clone()) {
                return Err(format!("duplicate tier id {}", tier.id));
            }
        }

        for key in &self.trusted_keys {
            if key.key_id.trim().is_empty() {
                return Err("every trusted key needs a key_id".to_string());
            }
            if decode_key(&key.public_key_base64).is_none() {
                return Err(format!(
                    "trusted key {} is not a valid base64-encoded 32-byte Ed25519 public key",
                    key.key_id
                ));
            }
        }

        Ok(())
    }

    /// The tier a licence payload names, or `None` when it is unknown to this build.
    pub fn tier(&self, tier_id: &str) -> Option<&ProductTier> {
        self.tiers.iter().find(|tier| tier.id == tier_id)
    }

    /// The first tier, used as the default when a payload does not name one.
    pub fn default_tier(&self) -> &ProductTier {
        &self.tiers[0]
    }

    /// The active verification key with this id, if it is still trusted.
    pub fn key(&self, key_id: &str) -> Option<&TrustedKey> {
        self.trusted_keys
            .iter()
            .find(|key| key.key_id == key_id && key.active)
    }

    /// The newest active key, used when signing (backend) and as the default when verifying.
    pub fn primary_key(&self) -> Option<&TrustedKey> {
        self.trusted_keys.iter().find(|key| key.active)
    }
}

/// Decodes a base64 public key into 32 bytes.
pub fn decode_key(encoded: &str) -> Option<[u8; 32]> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_configuration_is_valid() {
        assert_eq!(DEFAULT_CONFIG.validate(), Ok(()));
    }

    #[test]
    fn the_default_environment_is_test_mode() {
        // A build produced without an explicit release action must never mint a real entitlement.
        assert!(DEFAULT_CONFIG.environment.is_test());
    }

    #[test]
    fn the_lifetime_tier_is_permanent_and_has_no_update_deadline() {
        let tier = DEFAULT_CONFIG.default_tier();
        assert!(tier.permanent);
        assert_eq!(tier.update_eligibility_months, None);
        assert_eq!(tier.currency, "INR");
        assert_eq!(tier.price_minor_units, 149_900);
    }

    #[test]
    fn an_empty_product_id_is_rejected() {
        let mut config = DEFAULT_CONFIG.clone();
        config.product_id = "  ".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn a_zero_device_limit_is_rejected() {
        let mut config = DEFAULT_CONFIG.clone();
        config.device_limit = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn an_absurd_device_limit_is_rejected() {
        let mut config = DEFAULT_CONFIG.clone();
        config.device_limit = 500;
        assert!(config.validate().is_err());
    }

    #[test]
    fn a_zero_offline_lease_is_rejected() {
        let mut config = DEFAULT_CONFIG.clone();
        config.offline_lease_days = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn a_paid_tier_that_costs_nothing_is_rejected() {
        let mut config = DEFAULT_CONFIG.clone();
        config.tiers = vec![ProductTier {
            id: "free-pro".to_string(),
            name: "Free Pro".to_string(),
            price_minor_units: 0,
            currency: "INR".to_string(),
            permanent: true,
            update_eligibility_months: None,
            available: true,
        }];
        assert!(config.validate().is_err());
    }

    #[test]
    fn a_bad_currency_code_is_rejected() {
        let mut config = DEFAULT_CONFIG.clone();
        config.tiers = vec![ProductTier {
            id: "pro".to_string(),
            name: "Pro".to_string(),
            price_minor_units: 1000,
            currency: "rupees".to_string(),
            permanent: true,
            update_eligibility_months: None,
            available: true,
        }];
        assert!(config.validate().is_err());
    }

    #[test]
    fn duplicate_tier_ids_are_rejected() {
        let mut config = DEFAULT_CONFIG.clone();
        let tier = config.tiers[0].clone();
        config.tiers.push(tier);
        assert!(config.validate().is_err());
    }

    #[test]
    fn no_active_key_is_rejected() {
        let mut config = DEFAULT_CONFIG.clone();
        for key in config.trusted_keys.iter_mut() {
            key.active = false;
        }
        assert!(config.validate().is_err());
    }

    #[test]
    fn a_non_base64_key_is_rejected() {
        let mut config = DEFAULT_CONFIG.clone();
        config.trusted_keys = vec![TrustedKey {
            key_id: "k-bad".to_string(),
            public_key_base64: "not base64 at all!!".to_string(),
            active: true,
            note: String::new(),
        }];
        assert!(config.validate().is_err());
    }

    #[test]
    fn a_key_of_the_wrong_length_is_rejected() {
        let mut config = DEFAULT_CONFIG.clone();
        config.trusted_keys = vec![TrustedKey {
            key_id: "k-short".to_string(),
            // 16 bytes, not 32.
            public_key_base64: "AAAAAAAAAAAAAAAAAAAAAA==".to_string(),
            active: true,
            note: String::new(),
        }];
        assert!(config.validate().is_err());
    }

    #[test]
    fn key_lookup_ignores_inactive_keys() {
        let mut config = DEFAULT_CONFIG.clone();
        config.trusted_keys = vec![
            TrustedKey {
                key_id: "k-old".to_string(),
                public_key_base64: DEFAULT_CONFIG.trusted_keys[0].public_key_base64.to_string(),
                active: false,
                note: "rotated out".to_string(),
            },
            TrustedKey {
                key_id: "k-new".to_string(),
                public_key_base64: DEFAULT_CONFIG.trusted_keys[0].public_key_base64.to_string(),
                active: true,
                note: "current".to_string(),
            },
        ];
        assert!(config.key("k-old").is_none());
        assert!(config.key("k-new").is_some());
        assert_eq!(config.primary_key().unwrap().key_id, "k-new");
    }

    #[test]
    fn an_unknown_tier_is_not_found() {
        assert!(DEFAULT_CONFIG.tier("enterprise").is_none());
        assert!(DEFAULT_CONFIG.tier("pro-lifetime").is_some());
    }

    #[test]
    fn key_decoding_round_trips() {
        let encoded = DEFAULT_CONFIG.trusted_keys[0].public_key_base64;
        let bytes = decode_key(encoded).expect("a valid 32-byte key");
        assert_eq!(bytes.len(), 32);
        assert_eq!(decode_key(""), None);
        assert_eq!(decode_key("AAAA"), None);
    }

    #[test]
    fn serde_uses_camel_case() {
        let json = serde_json::to_string(&DEFAULT_CONFIG).unwrap();
        assert!(json.contains("\"productId\""));
        assert!(json.contains("\"deviceLimit\""));
        assert!(json.contains("\"offlineLeaseDays\""));
        assert!(json.contains("\"trustedKeys\""));
    }
}
