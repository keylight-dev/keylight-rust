//! Keylight licensing SDK for Rust.
//!
//! Activate and validate license keys against a Keylight Worker, with offline
//! Ed25519 lease verification. See the crate README for a quickstart.

pub mod lease;
pub use lease::Lease;

pub mod verifier;
pub use verifier::{verify_lease, VerifyResult, SKEW_SECONDS};
