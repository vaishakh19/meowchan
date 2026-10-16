//! Entitlement resolution.
//!
//! An `Entitlement` is what the UI is told. It is derived from a **verified** licence payload plus
//! the commercial configuration — never from anything the webview sent, and never from a
//! frontend boolean.
//!
//! The rules that shape this module:
//!   - verification is offline and local; no network call is required to scan,
//!   - any failure resolves to the free tier with a reason, so the app keeps working,
//!   - a *permanent* entitlement never expires; only the offline lease does,
//!   - the frontend reads the entitlement, it never decides it.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::product::{CommercialConfig, ProductTier};
use crate::licensing::payload::{LicensePayload, LicenseError};

/// Mirrors `PlanTier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanTier {
    Free,
    Trial,
    Pro,
}

impl PlanTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanTier::Free => "free",
            PlanTier::Trial => "trial",
            PlanTier::Pro => "pro",
        }
    }
}

/// Mirrors `EntitlementState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntitlementState {
    Free,
    Trial,
    ProActive,
    ProExpired,
    Invalid,
}

impl EntitlementState {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntitlementState::Free => "free",
            EntitlementState::Trial => "trial",
            EntitlementState::ProActive => "pro-active",
            EntitlementState::ProExpired => "pro-expired",
            EntitlementState::Invalid => "invalid",
        }
    }

    /// Parses a stored state string. Unknown values become `free` (fail closed).
    pub fn parse(value: &str) -> Self {
        match value {
            "trial" => EntitlementState::Trial,
            "pro-active" => EntitlementState::ProActive,
            "pro-expired" => EntitlementState::ProExpired,
            "invalid" => EntitlementState::Invalid,
            _ => EntitlementState::Free,
        }
    }

    /// True when paid features are available.
    pub fn is_paid(&self) -> bool {
        matches!(self, EntitlementState::Trial | EntitlementState::ProActive)
    }
}

/// Mirrors `FeatureFlags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlags {
    pub core_analysis: bool,
    pub scan_history: bool,
    pub advanced_duplicates: bool,
    pub custom_cleanup_rules: bool,
    pub scheduled_scans: bool,
    pub export_reports: bool,
    pub advanced_filters: bool,
}

impl FeatureFlags {
    /// The free tier is a complete product: scanning, visualization and manual cleanup always work.
    pub fn free() -> Self {
        FeatureFlags {
            core_analysis: true,
            scan_history: false,
            advanced_duplicates: false,
            custom_cleanup_rules: false,
            scheduled_scans: false,
            export_reports: false,
            advanced_filters: false,
        }
    }

    /// Trial and active Pro get everything.
    pub fn pro() -> Self {
        FeatureFlags {
            core_analysis: true,
            scan_history: true,
            advanced_duplicates: true,
            custom_cleanup_rules: true,
            scheduled_scans: true,
            export_reports: true,
            advanced_filters: true,
        }
    }

    pub fn for_state(state: EntitlementState) -> Self {
        if state.is_paid() {
            FeatureFlags::pro()
        } else {
            FeatureFlags::free()
        }
    }
}

/// Mirrors `Entitlement`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entitlement {
    pub state: EntitlementState,
    pub tier: PlanTier,
    pub features: FeatureFlags,
    /// Masked licence key, safe to show and safe to log.
    pub license_key_masked: Option<String>,
    /// The licence identifier, when a licence is active.
    pub license_id: Option<String>,
    /// `true` when the entitlement itself never expires (a one-time purchase).
    pub permanent: bool,
    /// When the offline validation lease ends, if it is time-limited.
    pub lease_until: Option<i64>,
    /// When the entitlement was last verified against the licence service.
    pub verified_at: Option<i64>,
    /// How many days of the offline lease remain. `None` for a perpetual lease.
    pub days_remaining: Option<i64>,
    /// How many devices this licence may be active on.
    pub device_limit: u32,
    /// How many devices are currently active.
    pub active_devices: u32,
    /// Where this entitlement came from: `default`, `license-key`, `cache`.
    pub source: String,
    /// A customer-facing reason when the tier is not what the customer expected.
    pub reason: Option<String>,
    /// Free-tier duplicate group limit. `0` means unlimited.
    pub duplicate_group_limit: u32,
}

/// Free-tier duplicate group limit.
pub const FREE_DUPLICATE_GROUP_LIMIT: u32 = 25;
/// How long a cached entitlement keeps working with no re-validation.
pub const OFFLINE_GRACE_DAYS: i64 = 30;

pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as i64)
        .unwrap_or_default()
}

/// The entitlement used when nothing is activated. Never blocks anything.
pub fn free_entitlement() -> Entitlement {
    Entitlement {
        state: EntitlementState::Free,
        tier: PlanTier::Free,
        features: FeatureFlags::free(),
        license_key_masked: None,
        license_id: None,
        permanent: false,
        lease_until: None,
        verified_at: None,
        days_remaining: None,
        device_limit: 1,
        active_devices: 0,
        source: "default".to_string(),
        reason: None,
        duplicate_group_limit: FREE_DUPLICATE_GROUP_LIMIT,
    }
}

/// Builds the entitlement for a **verified** payload.
///
/// The caller must have already checked the signature, the schema version, the product and the
/// device binding. This function only turns a trusted payload into something the UI can render.
pub fn entitlement_from_payload(
    payload: &LicensePayload,
    tier: &ProductTier,
    config: &CommercialConfig,
    now: i64,
) -> Entitlement {
    let paid = !payload.lease_lapsed(now);
    let state = if paid {
        if payload.tier_id.contains("trial") {
            EntitlementState::Trial
        } else {
            EntitlementState::ProActive
        }
    } else {
        // A lapsed lease on a permanent licence is a *re-verification* problem, not a loss of
        // ownership. It is reported distinctly so the UI can say so.
        EntitlementState::ProExpired
    };

    let days_remaining = payload
        .lease_until
        .map(|until| ((until - now) / 86_400_000).max(0));

    let reason = if !paid {
        Some(
            "This licence needs to be re-verified with the licence server. Connect to the internet and re-activate; your purchase is not affected."
                .to_string(),
        )
    } else {
        None
    };

    Entitlement {
        state,
        tier: if state.is_paid() {
            if state == EntitlementState::Trial {
                PlanTier::Trial
            } else {
                PlanTier::Pro
            }
        } else {
            PlanTier::Free
        },
        features: FeatureFlags::for_state(state),
        license_key_masked: None,
        license_id: Some(payload.license_id.clone()),
        permanent: payload.permanent && tier.permanent,
        lease_until: payload.lease_until,
        verified_at: Some(now),
        days_remaining,
        device_limit: payload.device_limit,
        active_devices: 0,
        source: "license-key".to_string(),
        reason,
        duplicate_group_limit: if state.is_paid() {
            0
        } else {
            config.free_duplicate_group_limit
        },
    }
}

/// Applies the offline policy to a cached entitlement.
///
/// The policy is deliberately narrow:
///   - an expired *lease* on a permanent licence downgrades to free and says why,
///   - a cached entitlement whose verification is older than the grace period downgrades to free,
///   - scanning is unaffected in every case.
pub fn apply_offline_grace(entitlement: Entitlement, now: i64) -> Entitlement {
    let mut entitlement = entitlement;
    if !entitlement.state.is_paid() {
        return entitlement;
    }

    if let Some(until) = entitlement.lease_until {
        if now >= until {
            entitlement.state = EntitlementState::ProExpired;
            entitlement.features = FeatureFlags::free();
            entitlement.reason = Some(
                "This licence needs to be re-verified with the licence server. Connect to the internet and re-activate; your purchase is not affected."
                    .to_string(),
            );
            entitlement.days_remaining = Some(0);
            return entitlement;
        }
    }

    let stale_after = entitlement
        .verified_at
        .unwrap_or(0)
        .saturating_add(OFFLINE_GRACE_DAYS * 86_400_000);
    if now > stale_after {
        entitlement.state = EntitlementState::Free;
        entitlement.tier = PlanTier::Free;
        entitlement.features = FeatureFlags::free();
        entitlement.reason = Some(
            "The licence could not be re-verified and the offline grace period has passed."
                .to_string(),
        );
        entitlement.days_remaining = None;
        entitlement.duplicate_group_limit = FREE_DUPLICATE_GROUP_LIMIT;
    }
    entitlement
}

/// A cached entitlement that could not be parsed. Fails closed.
pub fn from_cache(payload_json: &str) -> Entitlement {
    serde_json::from_str::<Entitlement>(payload_json).unwrap_or_else(|_| {
        let mut entitlement = free_entitlement();
        entitlement.state = EntitlementState::Invalid;
        entitlement.reason = Some(
            "The stored licence could not be read. Re-activate to restore your purchase.".to_string(),
        );
        entitlement
    })
}

/// The reason a licence key could not be used, rendered for a customer.
pub fn describe_error(error: &LicenseError) -> String {
    error.message()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::product::Environment;
    use crate::licensing::payload::{encode, payload_bytes};
    use ed25519_dalek::{Signer, SigningKey};

    const NOW: i64 = 1_700_000_100_000;

    fn config() -> CommercialConfig {
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
            trusted_keys: vec![],
        }
    }

    fn tier() -> ProductTier {
        ProductTier {
            id: "pro-lifetime".to_string(),
            name: "Diskwise Pro".to_string(),
            price_minor_units: 149_900,
            currency: "INR".to_string(),
            permanent: true,
            update_eligibility_months: None,
            available: true,
        }
    }

    fn payload() -> LicensePayload {
        LicensePayload::permanent(
            "lic-1",
            "diskwise-pro",
            "pro-lifetime",
            "k-test",
            NOW - 1000,
            3,
        )
    }

    #[test]
    fn the_free_entitlement_never_blocks_core_analysis() {
        let entitlement = free_entitlement();
        assert_eq!(entitlement.state, EntitlementState::Free);
        assert!(entitlement.features.core_analysis);
        assert_eq!(entitlement.duplicate_group_limit, FREE_DUPLICATE_GROUP_LIMIT);
        assert!(!entitlement.features.scan_history);
        assert!(!entitlement.permanent);
    }

    #[test]
    fn pro_features_include_everything() {
        let features = FeatureFlags::pro();
        assert!(features.core_analysis);
        assert!(features.scan_history);
        assert!(features.advanced_duplicates);
        assert!(features.custom_cleanup_rules);
        assert!(features.scheduled_scans);
        assert!(features.export_reports);
        assert!(features.advanced_filters);
    }

    #[test]
    fn a_verified_permanent_payload_produces_an_active_pro_entitlement() {
        let entitlement = entitlement_from_payload(&payload(), &tier(), &config(), NOW);
        assert_eq!(entitlement.state, EntitlementState::ProActive);
        assert!(entitlement.state.is_paid());
        assert!(entitlement.permanent);
        assert_eq!(entitlement.device_limit, 3);
        assert_eq!(entitlement.duplicate_group_limit, 0, "unlimited for Pro");
        assert_eq!(entitlement.license_id.as_deref(), Some("lic-1"));
        assert!(entitlement.reason.is_none());
    }

    #[test]
    fn a_trial_payload_produces_a_trial_entitlement() {
        let mut payload = payload();
        payload.tier_id = "pro-trial".to_string();
        let entitlement = entitlement_from_payload(&payload, &tier(), &config(), NOW);
        assert_eq!(entitlement.state, EntitlementState::Trial);
        assert_eq!(entitlement.tier, PlanTier::Trial);
    }

    #[test]
    fn a_lapsed_lease_on_a_permanent_licence_reports_a_reverification_problem() {
        let mut payload = payload();
        payload.lease_until = Some(NOW - 1);
        let entitlement = entitlement_from_payload(&payload, &tier(), &config(), NOW);
        assert_eq!(entitlement.state, EntitlementState::ProExpired);
        assert!(!entitlement.state.is_paid());
        // Ownership is still recorded, so the UI can say so.
        assert!(entitlement.permanent);
        assert!(entitlement.reason.unwrap().contains("purchase is not affected"));
    }

    #[test]
    fn state_and_tier_names_match_the_wire_format() {
        assert_eq!(EntitlementState::ProActive.as_str(), "pro-active");
        assert_eq!(EntitlementState::ProExpired.as_str(), "pro-expired");
        assert_eq!(EntitlementState::Invalid.as_str(), "invalid");
        assert_eq!(EntitlementState::parse("pro-active"), EntitlementState::ProActive);
        assert_eq!(EntitlementState::parse("nonsense"), EntitlementState::Free);
        assert_eq!(PlanTier::Pro.as_str(), "pro");
    }

    #[test]
    fn the_offline_grace_period_expires_a_stale_cached_licence() {
        let mut cached = entitlement_from_payload(&payload(), &tier(), &config(), NOW);
        cached.verified_at = Some(0);
        cached.lease_until = None;

        let fresh = apply_offline_grace(cached.clone(), 1_000);
        assert!(fresh.state.is_paid());

        let stale = apply_offline_grace(cached, NOW + OFFLINE_GRACE_DAYS * 86_400_000 + 1);
        assert_eq!(stale.state, EntitlementState::Free);
        assert!(!stale.features.scan_history);
        assert!(stale.reason.is_some());
    }

    #[test]
    fn an_expired_cached_lease_falls_back_to_free() {
        let mut cached = entitlement_from_payload(&payload(), &tier(), &config(), NOW);
        cached.verified_at = Some(NOW);
        cached.lease_until = Some(1_000);
        let resolved = apply_offline_grace(cached, NOW);
        assert_eq!(resolved.state, EntitlementState::ProExpired);
        assert!(!resolved.features.scan_history);
    }

    #[test]
    fn a_cached_entitlement_round_trips_through_json() {
        let entitlement = entitlement_from_payload(&payload(), &tier(), &config(), NOW);
        let json = serde_json::to_string(&entitlement).unwrap();
        let decoded: Entitlement = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, entitlement);
    }

    #[test]
    fn an_unparseable_cache_fails_closed_to_invalid() {
        let entitlement = from_cache("{not json");
        assert_eq!(entitlement.state, EntitlementState::Invalid);
        assert!(entitlement.reason.is_some());
        // Scanning still works.
        assert!(entitlement.features.core_analysis);
    }

    #[test]
    fn serde_uses_camel_case() {
        let json = serde_json::to_string(&free_entitlement()).unwrap();
        assert!(json.contains("\"licenseKeyMasked\""));
        assert!(json.contains("\"duplicateGroupLimit\""));
        assert!(json.contains("\"leaseUntil\""));
        assert!(!json.contains("license_key_masked"));
    }

    #[test]
    fn a_signed_key_can_be_encoded_and_reparsed() {
        // Guards the encode/parse round trip the backend relies on.
        let signing = SigningKey::from_bytes(&[3u8; 32]);
        let payload = payload();
        let key = encode(&payload, &signing.sign(&payload_bytes(&payload)).to_bytes());
        let parsed = crate::licensing::payload::parse(&key).expect("parses");
        assert_eq!(parsed.payload, payload);
    }
}
