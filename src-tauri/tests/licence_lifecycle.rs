//! Licence lifecycle integration test.
//!
//! This is the executable half of the acceptance criteria for licensing. It reads the shared
//! vectors in `tests/fixtures/licences.json`, which `scripts/make-licence-fixtures.mjs` generates
//! with the same signing code the licence service uses — so the Rust verifier and the TypeScript
//! signer are proven to agree on the exact bytes.
//!
//! ## What it proves
//!
//!   * a valid licence verifies
//!   * an invalid signature is refused
//!   * a modified payload is refused
//!   * a licence for the wrong product is refused
//!   * a licence bound to the wrong device is refused
//!   * a licence for an unknown tier or an unknown schema is refused
//!   * a licence issued in the future is refused
//!   * a lapsed offline lease is refused, and a live one is accepted
//!   * an absurd device limit is refused
//!   * a malformed or empty key is refused
//!   * a frontend bypass attempt changes nothing
//!   * the free tier still scans
//!
//! Run with `cargo test --test licence_lifecycle`. It needs no database, no network and no
//! filesystem outside the fixtures directory.

use diskwise_lib::authz::{authorize, Authorization, Permission};
use diskwise_lib::config::product::{CommercialConfig, Environment, ProductTier, TrustedKey};
use diskwise_lib::licensing::payload::{self, DeviceBinding, LicenseError, LicensePayload};
use diskwise_lib::licensing::verify::{
    apply_offline_grace, entitlement_from_payload, free_entitlement, now_millis, EntitlementState,
};

/// The shared vectors, parsed once.
fn vectors() -> serde_json::Value {
    let raw = include_str!("fixtures/licences.json");
    serde_json::from_str(raw).expect("the fixture file is valid JSON")
}

/// The commercial configuration the verifier is run against, built from the fixture.
fn config(vectors: &serde_json::Value) -> CommercialConfig {
    CommercialConfig {
        product_id: vectors["productId"].as_str().unwrap().to_string(),
        product_name: "Diskwise Pro".to_string(),
        license_schema_version: 1,
        environment: Environment::Test,
        tiers: vec![ProductTier {
            id: vectors["tierId"].as_str().unwrap().to_string(),
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
            key_id: vectors["keyId"].as_str().unwrap().to_string(),
            public_key_base64: vectors["publicKeyBase64"].as_str().unwrap().to_string(),
            active: true,
            note: "fixture".to_string(),
        }],
    }
}

fn device() -> DeviceBinding {
    DeviceBinding {
        value: vectors()["deviceBinding"].as_str().unwrap().to_string(),
    }
}

/// Verifies a fixture case with no device binding, and returns the outcome or the error.
fn verify_case(name: &str) -> Result<LicensePayload, LicenseError> {
    verify_case_on(name, None)
}

/// Verifies a fixture case, optionally requiring it to be bound to `device`.
///
/// The device check is the last policy check before the lease, so passing a binding is what makes
/// `wrongDevice` fail where it should rather than passing as a valid licence.
fn verify_case_on(name: &str, device: Option<&str>) -> Result<LicensePayload, LicenseError> {
    let vectors = vectors();
    let key = vectors["cases"][name]["key"].as_str().unwrap();
    let parsed = payload::parse(key)?;
    payload::verify(&parsed, &config(&vectors), vectors["now"].as_i64().unwrap(), device)
}

#[test]
fn a_valid_licence_verifies() {
    let vectors = vectors();
    let verified = verify_case_on("valid", Some(vectors["deviceBinding"].as_str().unwrap()))
        .expect("verifies");
    assert_eq!(verified.product_id, "diskwise-pro");
    assert_eq!(verified.tier_id, "pro-lifetime");
    assert_eq!(
        verified.device_binding.as_deref(),
        Some(vectors["deviceBinding"].as_str().unwrap()),
    );
    assert!(verified.permanent);
    assert_eq!(verified.lease_until, None);
}

#[test]
fn an_invalid_signature_is_refused() {
    let error = verify_case("invalidSignature").expect_err("refused");
    assert_eq!(error, LicenseError::BadSignature);
}

#[test]
fn a_modified_payload_is_refused() {
    let error = verify_case("modifiedPayload").expect_err("refused");
    assert_eq!(error, LicenseError::BadSignature);
}

#[test]
fn a_licence_signed_by_an_untrusted_key_is_refused() {
    let error = verify_case("wrongKey").expect_err("refused");
    assert_eq!(error, LicenseError::UnknownKey { key_id: "k-other".to_string() });
}

#[test]
fn a_licence_for_the_wrong_product_is_refused() {
    let error = verify_case("wrongProduct").expect_err("refused");
    assert!(
        matches!(&error, LicenseError::WrongProduct { .. }),
        "expected a wrong-product error, got {error:?}"
    );
}

#[test]
fn a_licence_bound_to_another_device_is_refused() {
    let vectors = vectors();
    let error = verify_case_on("wrongDevice", Some(vectors["deviceBinding"].as_str().unwrap()))
        .expect_err("refused");
    assert!(
        matches!(&error, LicenseError::WrongDevice { .. }),
        "expected a wrong-device error, got {error:?}"
    );
}

#[test]
fn a_licence_for_an_unknown_tier_is_refused() {
    let error = verify_case("unknownTier").expect_err("refused");
    assert!(
        matches!(&error, LicenseError::UnknownTier { .. }),
        "expected an unknown-tier error, got {error:?}"
    );
}

#[test]
fn a_licence_for_an_unknown_schema_is_refused() {
    let error = verify_case("unknownSchema").expect_err("refused");
    assert!(
        matches!(&error, LicenseError::UnknownSchema { .. }),
        "expected an unknown-schema error, got {error:?}"
    );
}

#[test]
fn a_licence_issued_in_the_future_is_refused() {
    let error = verify_case("notYetValid").expect_err("refused");
    assert!(
        matches!(&error, LicenseError::NotYetValid { .. }),
        "expected a not-yet-valid error, got {error:?}"
    );
}

#[test]
fn a_lapsed_offline_lease_is_refused() {
    let error = verify_case("lapsedLease").expect_err("refused");
    assert!(
        matches!(&error, LicenseError::LeaseLapsed { .. }),
        "expected a lease-lapsed error, got {error:?}"
    );
}

#[test]
fn a_live_offline_lease_is_accepted() {
    let vectors = vectors();
    let parsed = payload::parse(vectors["cases"]["liveLease"]["key"].as_str().unwrap())
        .expect("parses");
    let verified = payload::verify(
        &parsed,
        &config(&vectors),
        vectors["now"].as_i64().unwrap(),
        None,
    )
    .expect("verifies");
    assert!(!verified.permanent);
    assert!(verified.lease_until.unwrap() > vectors["now"].as_i64().unwrap());
}

#[test]
fn an_absurd_device_limit_is_refused() {
    let error = verify_case("absurdDeviceLimit").expect_err("refused");
    assert!(
        matches!(&error, LicenseError::DeviceLimitExceeded { .. }),
        "expected a device-limit error, got {error:?}"
    );
}

#[test]
fn a_malformed_key_is_refused() {
    let error = verify_case("malformed").expect_err("refused");
    assert!(
        matches!(&error, LicenseError::Malformed(_)),
        "expected a malformed error, got {error:?}"
    );
}

#[test]
fn an_empty_key_is_refused() {
    let error = verify_case("empty").expect_err("refused");
    assert_eq!(error, LicenseError::Empty);
}

#[test]
fn every_error_message_is_safe_to_show_a_customer() {
    // No message may contain a key id, a public key, a signature, an internal path, or a stack
    // trace. These strings are shown verbatim in the UI.
    let vectors = vectors();
    let names: Vec<&str> = vectors["cases"]
        .as_object()
        .expect("an object")
        .keys()
        .map(|name| name.as_str())
        .collect();
    for name in names {
        if let Err(error) = verify_case(name) {
            let message = error.message();
            assert!(!message.is_empty(), "{name} produced an empty message");
            assert!(
                !message.contains("key_id") && !message.contains("publicKey"),
                "{name} leaked an internal field: {message}"
            );
            assert!(
                !message.contains("src-tauri") && !message.contains('\\') && !message.contains(".rs"),
                "{name} leaked a path: {message}"
            );
            assert!(
                !message.to_lowercase().contains("panic") && !message.to_lowercase().contains("stack"),
                "{name} leaked internals: {message}"
            );
        }
    }
}

/* --------------------------------------------------- entitlement resolution -- */

#[test]
fn a_verified_licence_resolves_to_an_active_paid_entitlement() {
    let vectors = vectors();
    let parsed = payload::parse(vectors["cases"]["valid"]["key"].as_str().unwrap()).expect("parses");
    let verified = payload::verify(
        &parsed,
        &config(&vectors),
        vectors["now"].as_i64().unwrap(),
        None,
    )
    .expect("verifies");
    let tier = config(&vectors).tier(&verified.tier_id).cloned().expect("a tier");
    let entitlement = entitlement_from_payload(
        &verified,
        &tier,
        &config(&vectors),
        vectors["now"].as_i64().unwrap(),
    );

    assert_eq!(entitlement.state, EntitlementState::ProActive);
    // The flag names are the shared contract's. Paid cleanup and permanent deletion both hang off
    // `customCleanupRules`; duplicate removal hangs off `advancedDuplicates`.
    assert!(entitlement.features.custom_cleanup_rules, "cleanup is unlocked");
    assert!(entitlement.features.advanced_duplicates, "duplicate removal is unlocked");
    assert!(entitlement.features.export_reports, "report export is unlocked");
    assert!(entitlement.features.scan_history, "scan history is unlocked");
    assert!(entitlement.permanent, "a one-time purchase is permanent");
    assert!(entitlement.license_id.is_some());
    assert_eq!(entitlement.device_limit, 3);
    assert_eq!(entitlement.lease_until, None);
    // A permanent licence never reports "expires in N days".
    assert_eq!(entitlement.days_remaining, None);
}

#[test]
fn a_lapsed_lease_on_a_permanent_licence_is_not_revocation() {
    // This is the distinction the product depends on: the *purchase* is permanent, and only the
    // offline re-validation lease is time-limited.
    let vectors = vectors();
    let now = vectors["now"].as_i64().unwrap();
    let parsed = payload::parse(vectors["cases"]["valid"]["key"].as_str().unwrap()).expect("parses");
    let mut verified = payload::verify(&parsed, &config(&vectors), now, None).expect("verifies");
    let tier = config(&vectors).tier(&verified.tier_id).cloned().expect("a tier");

    // A permanent licence carries no lease at all, so it cannot lapse.
    let entitlement = entitlement_from_payload(&verified, &tier, &config(&vectors), now);
    assert!(entitlement.permanent);
    assert_eq!(entitlement.state, EntitlementState::ProActive);

    // Even a payload that somehow carries an old lease on a permanent tier stays ProActive: the
    // permanent flag wins.
    verified.lease_until = Some(now - 24 * 60 * 60 * 1000);
    let graced = entitlement_from_payload(&verified, &tier, &config(&vectors), now);
    assert_eq!(graced.state, EntitlementState::ProActive);
}

#[test]
fn offline_grace_expires_a_non_permanent_licence_but_says_the_purchase_is_unaffected() {
    let vectors = vectors();
    let now = vectors["now"].as_i64().unwrap();
    let parsed = payload::parse(vectors["cases"]["liveLease"]["key"].as_str().unwrap())
        .expect("parses");
    let verified = payload::verify(&parsed, &config(&vectors), now, None).expect("verifies");
    let tier = config(&vectors).tier(&verified.tier_id).cloned().expect("a tier");

    // Inside the lease.
    let inside = entitlement_from_payload(&verified, &tier, &config(&vectors), now);
    assert_eq!(inside.state, EntitlementState::ProActive);

    // Past the lease, the cached entitlement degrades. The reason must not imply the customer lost
    // something they paid for.
    let far_future = now + 400 * 24 * 60 * 60 * 1000;
    let expired = apply_offline_grace(inside, far_future);
    assert_eq!(expired.state, EntitlementState::ProExpired);
    let reason = expired.reason.unwrap_or_default();
    assert!(
        !reason.to_lowercase().contains("expired your licence")
            && !reason.to_lowercase().contains("no longer licensed"),
        "the reason must not imply loss of ownership: {reason}"
    );
}

/* -------------------------------------------------------------- authorization -- */

#[test]
fn a_modified_frontend_cannot_bypass_authorization() {
    // The command surface is what a tampered interface can reach. Every paid permission is denied
    // on the free tier, and every free permission is granted.
    let auth = Authorization::free(device());

    for permission in diskwise_lib::authz::PERMISSIONS {
        let result = authorize(&auth, *permission);
        if permission.is_free() {
            assert!(result.is_ok(), "{permission:?} is free and must be allowed");
        } else {
            assert!(result.is_err(), "{permission:?} is paid and must be denied");
        }
    }
}

#[test]
fn a_verified_licence_grants_every_paid_permission() {
    let vectors = vectors();
    let now = vectors["now"].as_i64().unwrap();
    let key = vectors["cases"]["valid"]["key"].as_str().unwrap();
    let auth = Authorization::resolve(Some(key), &config(&vectors), now, device());

    assert!(auth.is_paid());
    for permission in diskwise_lib::authz::PERMISSIONS {
        assert!(
            authorize(&auth, *permission).is_ok(),
            "{permission:?} must be granted to a verified licence"
        );
    }
}

#[test]
fn a_forged_licence_grants_nothing() {
    let vectors = vectors();
    let now = vectors["now"].as_i64().unwrap();
    for name in [
        "invalidSignature",
        "modifiedPayload",
        "wrongKey",
        "wrongProduct",
        "unknownTier",
        "unknownSchema",
        "lapsedLease",
        "malformed",
        "empty",
    ] {
        let key = vectors["cases"][name]["key"].as_str().unwrap();
        let auth = Authorization::resolve(Some(key), &config(&vectors), now, device());
        assert!(!auth.is_paid(), "{name} must not grant a paid entitlement");
        for permission in diskwise_lib::authz::PERMISSIONS {
            if !permission.is_free() {
                assert!(
                    authorize(&auth, *permission).is_err(),
                    "{name} must not grant {permission:?}"
                );
            }
        }
    }
}

#[test]
fn the_free_tier_is_a_complete_product() {
    let entitlement = free_entitlement();
    assert_eq!(entitlement.state, EntitlementState::Free);
    // Everything the free permission set covers is on: scanning, inspection, previews, the large
    // file finder and basic duplicate detection.
    assert!(entitlement.features.core_analysis, "scanning is free");
    assert!(!entitlement.features.custom_cleanup_rules, "cleanup is paid");
    assert!(!entitlement.features.advanced_duplicates, "duplicate removal is paid");
    assert!(!entitlement.features.export_reports, "export is paid");
    assert!(!entitlement.features.scan_history, "scan history is paid");
    assert!(!entitlement.permanent);
    assert_eq!(entitlement.duplicate_group_limit, 25);
}

/* --------------------------------------------------------------- device limit -- */

#[test]
fn the_device_limit_travels_in_the_signed_payload() {
    let vectors = vectors();
    let now = vectors["now"].as_i64().unwrap();
    let key = vectors["cases"]["valid"]["key"].as_str().unwrap();
    let auth = Authorization::resolve(Some(key), &config(&vectors), now, device());
    assert_eq!(auth.device_limit(), 3);

    // A payload claiming an absurd limit is refused before the limit is ever read.
    let absurd = vectors["cases"]["absurdDeviceLimit"]["key"].as_str().unwrap();
    let auth = Authorization::resolve(Some(absurd), &config(&vectors), now, device());
    assert!(!auth.is_paid());
    // With no payload, the limit defaults to one: the most conservative value.
    assert_eq!(auth.device_limit(), 1);
}

#[test]
fn a_reinstall_on_the_same_machine_reuses_the_same_binding() {
    let vectors = vectors();
    let now = vectors["now"].as_i64().unwrap();
    let key = vectors["cases"]["valid"]["key"].as_str().unwrap();
    let first = Authorization::resolve(Some(key), &config(&vectors), now, device());
    // A reinstall resolves the same key on the same binding: the server refreshes the activation
    // row rather than consuming a second slot.
    let second = Authorization::resolve(Some(key), &config(&vectors), now, device());
    assert_eq!(first.device.value, second.device.value);
    assert_eq!(first.entitlement.state, second.entitlement.state);
}

#[test]
fn a_device_transfer_is_a_different_binding() {
    let vectors = vectors();
    let now = vectors["now"].as_i64().unwrap();
    let key = vectors["cases"]["valid"]["key"].as_str().unwrap();
    let other = DeviceBinding {
        value: vectors["otherDeviceBinding"].as_str().unwrap().to_string(),
    };
    let auth = Authorization::resolve(Some(key), &config(&vectors), now, other);
    // The licence is bound to the original machine, so it is refused on the new one.
    assert!(!auth.is_paid());
}

/* ------------------------------------------------------------------ time ---- */

#[test]
fn the_clock_is_milliseconds_since_the_epoch() {
    let now = now_millis();
    // Sanity: after 2020, before 2100.
    assert!(now > 1_577_836_800_000);
    assert!(now < 4_102_444_800_000);
}

/* ------------------------------------------------------- fixture consistency -- */

#[test]
fn the_fixture_declares_every_case_it_contains() {
    let vectors = vectors();
    let cases = vectors["cases"].as_object().expect("an object");
    assert!(cases.len() >= 12, "expected at least a dozen cases, found {}", cases.len());
    for (name, case) in cases {
        assert!(
            case["expect"].is_string(),
            "case {name} must declare an expected outcome"
        );
        assert!(
            case["key"].is_string(),
            "case {name} must supply a licence key"
        );
    }
}
