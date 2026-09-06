//! Ed25519 verification of server-owned product settings.
//!
//! The payload format is frozen across every SDK — see section 4 of
//! `keylight-cpp/docs/superpowers/specs/2026-09-05-trial-parity-handoff.md`:
//!
//! ```text
//! cfg1|{kid}|{tenantId}|{productId}|{issuedAt}|{expiresAt}|{trialDurationDays}|{freeTierEnabled}
//! ```
//!
//! Once a verifying client is in the wild these bytes cannot change: every
//! shipped client would reject valid configs, and it cannot be fixed from the
//! server. The first test is therefore a golden vector captured from the live
//! worker rather than a fixture this file generated for itself — a fixture only
//! proves the file agrees with itself. It is the same vector the Swift and C#
//! SDKs pin.

use std::collections::HashMap;

use keylight::{verify_config, ProductConfigFields};

// Captured from GET https://api.keylight.dev/anotheragence/getbarry/config on
// 2026-09-06, worker version 717cfb7c. The public key is what
// GET /anotheragence/.well-known/keylight-keys serves for kid k1.
const TENANT_ID: &str = "anotheragence";
const PRODUCT_ID: &str = "getbarry";
const KID: &str = "k1";
const PUBLIC_KEY: &str = "wPOiRNiP2hbc0O4UCAuO6FRRLKp4YvGtf8V27xnPzNY=";
const SIGNATURE: &str =
    "WbyLOjmB7jtA3Ny9Qon/uJtXpZx61/Vx+U7OsQpSD17xkem5QrYwvSQOmRLw7J6Ozhgr8r2bptQ/UhiDRUZ7DA==";
const ISSUED_AT: i64 = 1_788_695_996;
const EXPIRES_AT: i64 = 1_788_782_396;
const SKEW: i64 = 300;

/// An instant inside the signature window, so freshness passes deterministically
/// forever rather than for one day in 2026.
const INSIDE_WINDOW: i64 = 1_788_700_000;

fn trusted(kid: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert(kid.to_string(), PUBLIC_KEY.to_string());
    m
}

fn fields(trial_days: Option<u32>, free_tier: Option<bool>) -> ProductConfigFields {
    ProductConfigFields {
        trial_duration_days: trial_days,
        free_tier_enabled: free_tier,
        issued_at: Some(ISSUED_AT),
        expires_at: Some(EXPIRES_AT),
        kid: Some(KID.to_string()),
        signature: Some(SIGNATURE.to_string()),
    }
}

fn golden() -> ProductConfigFields {
    fields(Some(1), Some(true))
}

fn verify_at(f: &ProductConfigFields, tenant: &str, product: &str, now: i64, kid: &str) -> bool {
    verify_config(f, tenant, product, &trusted(kid), now, SKEW)
}

#[test]
fn verifies_a_real_signature_from_the_live_worker() {
    assert!(
        verify_at(&golden(), TENANT_ID, PRODUCT_ID, INSIDE_WINDOW, KID),
        "Rust disagrees with the bytes the deployed worker signs"
    );
}

// Rule 1: tenant and product come from local config, never the body.

#[test]
fn rejects_a_config_signed_for_a_different_product() {
    assert!(!verify_at(&golden(), TENANT_ID, "someotherapp", INSIDE_WINDOW, KID));
}

#[test]
fn rejects_a_config_signed_for_a_different_tenant() {
    assert!(!verify_at(&golden(), "someothertenant", PRODUCT_ID, INSIDE_WINDOW, KID));
}

#[test]
fn rejects_a_kid_missing_from_the_trusted_keyset() {
    assert!(
        !verify_at(&golden(), TENANT_ID, PRODUCT_ID, INSIDE_WINDOW, "k2"),
        "a rotated-away kid must be rejected even when the signature itself is valid"
    );
}

#[test]
fn rejects_tampered_values() {
    let longer = fields(Some(365), Some(true));
    assert!(
        !verify_at(&longer, TENANT_ID, PRODUCT_ID, INSIDE_WINDOW, KID),
        "the whole point: a longer trial must not verify"
    );
}

#[test]
fn rejects_an_absent_signature() {
    let mut unsigned = golden();
    unsigned.signature = None;
    assert!(!verify_at(&unsigned, TENANT_ID, PRODUCT_ID, INSIDE_WINDOW, KID));
}

// Rule 3: freshness applies to the wire.

#[test]
fn rejects_a_response_from_before_its_issued_at() {
    assert!(!verify_at(&golden(), TENANT_ID, PRODUCT_ID, ISSUED_AT - SKEW - 1, KID));
}

#[test]
fn rejects_a_response_past_its_expiry() {
    assert!(!verify_at(&golden(), TENANT_ID, PRODUCT_ID, EXPIRES_AT + SKEW + 1, KID));
}

#[test]
fn accepts_clock_drift_within_the_skew_tolerance() {
    assert!(verify_at(&golden(), TENANT_ID, PRODUCT_ID, EXPIRES_AT + SKEW - 1, KID));
    assert!(verify_at(&golden(), TENANT_ID, PRODUCT_ID, ISSUED_AT - SKEW + 1, KID));
}

/// The worker signs a complete pair or sends the response unsigned, so a partial
/// config was never signable and must not be reconstructed as if it were.
#[test]
fn rejects_a_partial_config() {
    let partial = fields(Some(1), None);
    assert!(!verify_at(&partial, TENANT_ID, PRODUCT_ID, INSIDE_WINDOW, KID));
}
