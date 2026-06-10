//! Keylight licensing SDK for Rust.
//!
//! Activate and validate license keys against a Keylight Worker, with offline
//! Ed25519 lease verification. See the crate README for a quickstart.

pub mod http;

pub mod error;
pub use error::{KeylightError, Result};

pub mod config;
pub use config::{KeylightConfig, KeylightConfigBuilder};

pub mod keyset;
pub use keyset::parse_keyset;

pub mod lease;
pub use lease::Lease;

pub mod verifier;
pub use verifier::{verify_lease, VerifyResult, SKEW_SECONDS};

pub mod telemetry;

pub mod store;
pub use store::LicenseStore;

pub mod state;
pub use state::{LicenseState, TrialStatus, KeylessState, LicenseLifecycleEvent, resolve_state};

pub mod clock;
pub use clock::clock_manipulated;
