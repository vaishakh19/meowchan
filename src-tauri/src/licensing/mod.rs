//! Entitlement and licensing.
//!
//! Rules:
//!   - verification is **offline** and **local**: a signed payload checked against a public key that
//!     ships in the binary. No private key, no payment secret, no network call is required to scan,
//!   - a licence check can never block scanning: any failure falls back to the free tier, which is a
//!     complete product,
//!   - the cached entitlement is honoured offline and expires on its own schedule,
//!   - the frontend never decides entitlement: it reads what this module produces.
//!
//! Online re-validation is intentionally opt-in and off by default; nothing here phones home.

pub mod keys;
pub mod verify;

pub use keys::{public_key, LICENSE_PUBLIC_KEY_BASE64};
pub use verify::{
    entitlement_from_license, free_entitlement, Entitlement, EntitlementState, FeatureFlags,
    PlanTier, LicensePayload,
};
