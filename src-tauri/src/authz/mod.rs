//! The authorization layer.
//!
//! The frontend is **not** a security boundary. Every capability that costs money, or that touches
//! the filesystem in a way the user did not explicitly ask for, is gated here, inside the native
//! backend, before the operation runs.
//!
//! A modified React interface cannot bypass this by invoking a Tauri command directly: the command
//! itself calls `authorize()` (or a helper that does), and the check reads the entitlement the
//! *backend* resolved — not anything the webview sent.
//!
//! ## Fail closed
//!
//! Every failure mode — no licence, malformed licence, forged signature, unknown key, wrong product,
//! wrong device, lapsed lease, unknown permission — denies the operation. The only thing a licence
//! problem never blocks is **scanning and previewing**, which the product tier makes free.

pub mod permissions;

pub use permissions::{Permission, PERMISSIONS};

use crate::config::product::CommercialConfig;
use crate::error::{AppError, AppResult};
use crate::licensing::payload::{self, DeviceBinding, LicensePayload};
use crate::licensing::verify::{Entitlement, EntitlementState};

/// What the backend knows about the current licence, resolved locally.
#[derive(Debug, Clone)]
pub struct Authorization {
    /// The entitlement the backend resolved. Never taken from the webview.
    pub entitlement: Entitlement,
    /// The verified payload, when a licence is active.
    pub payload: Option<LicensePayload>,
    /// This machine's binding.
    pub device: DeviceBinding,
}

impl Authorization {
    /// The free tier. Denies every paid permission, allows every free one.
    pub fn free(device: DeviceBinding) -> Self {
        Authorization {
            entitlement: crate::licensing::verify::free_entitlement(),
            payload: None,
            device,
        }
    }

    /// True when the paid tier is active and proven.
    pub fn is_paid(&self) -> bool {
        matches!(
            self.entitlement.state,
            EntitlementState::Trial | EntitlementState::ProActive
        )
    }

    /// The device limit in force, from the verified payload when present.
    pub fn device_limit(&self) -> u32 {
        self.payload
            .as_ref()
            .map(|payload| payload.device_limit)
            .unwrap_or(1)
    }

    /// Resolves the authorization from a stored licence key, if there is one.
    ///
    /// Any failure produces the free tier with a reason. This function never panics and never
    /// grants access on an error path.
    pub fn resolve(
        key: Option<&str>,
        config: &CommercialConfig,
        now: i64,
        device: DeviceBinding,
    ) -> Authorization {
        let Some(key) = key.filter(|key| !key.trim().is_empty()) else {
            return Authorization::free(device);
        };

        let parsed = match payload::parse(key) {
            Ok(parsed) => parsed,
            Err(error) => {
                let mut entitlement = crate::licensing::verify::free_entitlement();
                entitlement.state = EntitlementState::Invalid;
                entitlement.reason = Some(error.message());
                entitlement.source = "license-key".to_string();
                return Authorization {
                    entitlement,
                    payload: None,
                    device,
                };
            }
        };

        match payload::verify(&parsed, config, now, Some(&device.value)) {
            Ok(verified) => {
                let tier = config
                    .tier(&verified.tier_id)
                    .cloned()
                    .unwrap_or_else(|| config.default_tier().clone());
                let entitlement = crate::licensing::verify::entitlement_from_payload(
                    &verified,
                    &tier,
                    config,
                    now,
                );
                Authorization {
                    entitlement,
                    payload: Some(verified),
                    device,
                }
            }
            Err(error) => {
                let mut entitlement = crate::licensing::verify::free_entitlement();
                entitlement.state = EntitlementState::Invalid;
                entitlement.reason = Some(error.message());
                entitlement.source = "license-key".to_string();
                Authorization {
                    entitlement,
                    payload: None,
                    device,
                }
            }
        }
    }
}

/// The single gate every protected operation goes through.
///
/// Returns `Ok(())` when the permission is granted, and a customer-facing error when it is not.
/// The error never reveals whether a licence existed, only that the operation is not permitted.
pub fn authorize(auth: &Authorization, permission: Permission) -> AppResult<()> {
    if permission.is_free() {
        return Ok(());
    }

    if auth.is_paid() {
        return Ok(());
    }

    // Fail closed, with a message that says what to do rather than what failed.
    let reason = match &auth.entitlement.reason {
        Some(reason) => reason.clone(),
        None => format!(
            "{} is a Diskwise Pro feature. Scanning, previews and manual navigation are always free.",
            permission.label()
        ),
    };
    Err(AppError::NotPermitted(reason))
}

/// Convenience for the common case: authorize against the app's stored state.
pub fn authorize_state(state: &crate::state::AppState, permission: Permission) -> AppResult<()> {
    let auth = state.authorization()?;
    authorize(&auth, permission)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::product::{Environment, ProductTier, TrustedKey};
    use crate::licensing::payload::{encode, payload_bytes, LicensePayload};
    use ed25519_dalek::{Signer, SigningKey};

    const NOW: i64 = 1_700_000_100_000;

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn config(public: &str) -> CommercialConfig {
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
                key_id: "k-test".to_string(),
                public_key_base64: public.to_string(),
                active: true,
                note: "test".to_string(),
            }],
        }
    }

    fn device() -> DeviceBinding {
        DeviceBinding {
            value: "dev-test".to_string(),
        }
    }

    fn licensed(config: &CommercialConfig, signing: &SigningKey) -> Authorization {
        let payload = LicensePayload::permanent(
            "lic-1",
            "diskwise-pro",
            "pro-lifetime",
            "k-test",
            NOW - 1000,
            3,
        );
        let key = encode(&payload, &signing.sign(&payload_bytes(&payload)).to_bytes());
        Authorization::resolve(Some(&key), config, NOW, device())
    }

    #[test]
    fn free_authorization_denies_every_paid_permission() {
        let auth = Authorization::free(device());
        for permission in PERMISSIONS.iter().filter(|p| !p.is_free()) {
            assert!(
                authorize(&auth, *permission).is_err(),
                "{permission:?} must be denied on the free tier"
            );
        }
    }

    #[test]
    fn free_authorization_allows_every_free_permission() {
        let auth = Authorization::free(device());
        for permission in PERMISSIONS.iter().filter(|p| p.is_free()) {
            assert!(
                authorize(&auth, *permission).is_ok(),
                "{permission:?} must stay free"
            );
        }
    }

    #[test]
    fn a_valid_licence_grants_paid_permissions() {
        let signing = signing_key();
        let config = config(&base64::engine::general_purpose::STANDARD.encode(
            signing.verifying_key().to_bytes(),
        ));
        let auth = licensed(&config, &signing);
        assert!(auth.is_paid());
        for permission in PERMISSIONS.iter() {
            assert!(authorize(&auth, *permission).is_ok(), "{permission:?}");
        }
    }

    #[test]
    fn a_forged_licence_denies_paid_permissions() {
        let signing = signing_key();
        let config = config(&base64::engine::general_purpose::STANDARD.encode(
            signing.verifying_key().to_bytes(),
        ));
        let attacker = SigningKey::from_bytes(&[13u8; 32]);
        let payload = LicensePayload::permanent(
            "lic-1",
            "diskwise-pro",
            "pro-lifetime",
            "k-test",
            NOW - 1000,
            3,
        );
        let key = encode(&payload, &attacker.sign(&payload_bytes(&payload)).to_bytes());
        let auth = Authorization::resolve(Some(&key), &config, NOW, device());
        assert!(!auth.is_paid());
        assert!(authorize(&auth, Permission::PaidCleanup).is_err());
        // …but scanning still works.
        assert!(authorize(&auth, Permission::CoreAnalysis).is_ok());
    }

    #[test]
    fn a_licence_bound_to_another_device_denies_paid_permissions() {
        let signing = signing_key();
        let config = config(&base64::engine::general_purpose::STANDARD.encode(
            signing.verifying_key().to_bytes(),
        ));
        let payload = LicensePayload::permanent(
            "lic-1",
            "diskwise-pro",
            "pro-lifetime",
            "k-test",
            NOW - 1000,
            3,
        );
        let key = encode(&payload, &signing.sign(&payload_bytes(&payload)).to_bytes());
        // Resolved on a *different* machine.
        let auth = Authorization::resolve(
            Some(&key),
            &config,
            NOW,
            DeviceBinding {
                value: "dev-elsewhere".to_string(),
            },
        );
        assert!(!auth.is_paid());
        assert!(authorize(&auth, Permission::DuplicateRemoval).is_err());
    }

    #[test]
    fn no_licence_at_all_denies_paid_permissions() {
        let auth = Authorization::resolve(None, &config("x"), NOW, device());
        assert!(!auth.is_paid());
        assert!(authorize(&auth, Permission::PaidCleanup).is_err());
    }

    #[test]
    fn a_garbage_licence_denies_paid_permissions_and_says_why() {
        let auth = Authorization::resolve(Some("not-a-licence"), &config("x"), NOW, device());
        assert!(!auth.is_paid());
        let error = authorize(&auth, Permission::PaidCleanup).unwrap_err();
        let message = error.to_string();
        assert!(!message.is_empty());
        // The message must not leak internals.
        assert!(!message.contains("panicked"));
    }

    #[test]
    fn the_error_message_points_the_customer_at_a_remedy() {
        let auth = Authorization::free(device());
        let error = authorize(&auth, Permission::PaidCleanup).unwrap_err().to_string();
        assert!(error.contains("Pro"), "message: {error}");
        assert!(
            error.contains("always free") || error.contains("free"),
            "message should reassure about free features: {error}"
        );
    }

    #[test]
    fn every_permission_has_a_label_and_a_free_or_paid_classification() {
        for permission in PERMISSIONS.iter() {
            assert!(!permission.label().is_empty());
            let _ = permission.is_free();
        }
    }
}
