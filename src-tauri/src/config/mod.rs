//! Centralised commercial configuration.
//!
//! Every commercial value — prices, device limits, entitlement periods, tier definitions, the
//! signing-key set and the payment environment — lives here and nowhere else. The frontend reads
//! what the backend publishes; it never hardcodes a price, a device limit or an expiry.
//!
//! The configuration is validated on load and the app **fails closed**: an invalid configuration
//! produces the free tier rather than an accidental grant.

pub mod product;

pub use product::{
    CommercialConfig, Environment, ProductTier, TrustedKey, DEFAULT_CONFIG,
};
