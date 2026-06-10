//! # Keylight
//!
//! Open-source Rust SDK for [Keylight](https://keylight.dev) — license your Rust apps,
//! CLIs, daemons, and Tauri desktop apps with online activation and **offline Ed25519
//! license verification**.
//!
//! The API is **synchronous and runtime-free**: blocking HTTP (`ureq`), no `async`, and no
//! background threads. You drive re-validation on launch and on app events with
//! [`Keylight::check_on_launch`] / [`Keylight::refresh_if_needed`].
//!
//! ## Quickstart
//!
//! ```no_run
//! use keylight::{Keylight, KeylightConfig};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Build a config and fetch the tenant's trusted Ed25519 keyset so leases can be
//! // verified offline (or pin keys with `.trusted_key(kid, pub_b64)`).
//! let mut cfg = KeylightConfig::builder("your-tenant", "your-product")
//!     .max_offline_days(7)
//!     .build();
//! if let Some((_, keys)) = keylight::keyset::fetch_keyset(
//!     &keylight::http::ureq_transport::UreqTransport::default(),
//!     &cfg.base_url,
//!     &cfg.tenant_id,
//! ) {
//!     cfg.trusted_keys.extend(keys);
//! }
//!
//! let kl = Keylight::new(cfg)?;
//! let result = kl.activate("USER-LICENSE-KEY")?;
//! if result.activated && kl.has_entitlement("pro") {
//!     println!("Pro features unlocked");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## How it works
//!
//! - **Activation** ([`Keylight::activate`]) exchanges a license key for a signed `v3`
//!   [`Lease`]. The lease is Ed25519-verified ([`verify_lease`]) against the trusted keyset
//!   *before* anything is persisted.
//! - **Offline**, the cached lease is the single source of truth: [`Keylight::has_entitlement`]
//!   and [`Keylight::state`] resolve from it with a [`SKEW_SECONDS`]-second clock-skew
//!   tolerance and an optional `max_offline_days` grace window.
//! - **Storage** is device-bound and encrypted with ChaCha20-Poly1305
//!   ([`EncryptedFileStore`]); both the store ([`LicenseStore`]) and the HTTP transport
//!   ([`http::Transport`]) are swappable traits for tests or custom platforms.
//!
//! ## Feature map
//!
//! | Area | Entry points |
//! |------|--------------|
//! | Lifecycle | [`Keylight::activate`], [`Keylight::validate`], [`Keylight::deactivate`] |
//! | Offline refresh | [`Keylight::refresh_if_needed`], [`Keylight::check_on_launch`] |
//! | State & entitlements | [`Keylight::state`], [`LicenseState`], [`Keylight::has_entitlement`] |
//! | Trials & free tier | [`Keylight::start_trial`], [`Keylight::check_trial`], [`Keylight::report_keyless_state`] |
//! | Events | [`Keylight::with_event_handler`], [`LicenseLifecycleEvent`] |
//! | Offline verification | [`verify_lease`], [`Lease`], [`SKEW_SECONDS`] |
//!
//! The lease verifier is gated by Keylight's frozen SP-0 conformance vectors, keeping
//! offline verification behavior identical across the Keylight SDK family.

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
pub use store::device::{DeviceIdentity, FixedDeviceIdentity, SystemDeviceIdentity};
pub use store::encrypted_file::EncryptedFileStore;
pub use store::LicenseStore;

pub mod client;
pub use client::{ActivationResult, Keylight, ValidationResult};

pub mod state;
pub use state::{
    lifecycle_event, resolve_state, KeylessState, LicenseLifecycleEvent, LicenseState, TrialStatus,
};

pub mod clock;
pub use clock::clock_manipulated;
