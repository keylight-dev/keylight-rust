//! Ed25519 lease verification with a fixed clock-skew tolerance ([`SKEW_SECONDS`]).
//! This module is gated by the SP-0 conformance vectors.

use crate::lease::Lease;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier as _, VerifyingKey};
use std::collections::HashMap;

pub const SKEW_SECONDS: i64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyResult {
    pub kid_known: bool,
    pub signature_valid: bool,
    pub expired: bool,
}

impl VerifyResult {
    /// The lease is signed by a known, trusted key (independent of expiry).
    pub fn is_trusted(&self) -> bool {
        self.kid_known && self.signature_valid
    }
}

/// Decode standard or url-safe base64, tolerating missing padding.
fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let norm = s
        .replace(|c: char| c.is_whitespace(), "")
        .replace('-', "+")
        .replace('_', "/");
    let padded = match norm.len() % 4 {
        0 => norm,
        n => format!("{norm}{}", "=".repeat(4 - n)),
    };
    base64::engine::general_purpose::STANDARD
        .decode(padded.as_bytes())
        .ok()
}

/// Verify a lease against a trusted `kid -> raw ed25519 public key (base64)` map.
pub fn verify_lease(
    lease: &Lease,
    trusted_keys: &HashMap<String, String>,
    now_seconds: i64,
    skew_seconds: i64,
) -> VerifyResult {
    let expired = now_seconds > lease.expires_at + skew_seconds;
    let pub_b64 = match trusted_keys.get(&lease.kid) {
        Some(p) => p,
        None => {
            return VerifyResult {
                kid_known: false,
                signature_valid: false,
                expired,
            };
        }
    };
    let signature_valid = (|| -> Option<bool> {
        let pk_bytes: [u8; 32] = b64_decode(pub_b64)?.try_into().ok()?;
        let vk = VerifyingKey::from_bytes(&pk_bytes).ok()?;
        let sig = Signature::from_slice(&b64_decode(&lease.signature)?).ok()?;
        Some(vk.verify(lease.payload().as_bytes(), &sig).is_ok())
    })()
    .unwrap_or(false);
    VerifyResult {
        kid_known: true,
        signature_valid,
        expired,
    }
}

/// The canonical preimage for a signed product config.
///
/// Frozen across every SDK. Once a verifying client is in the wild these bytes
/// cannot change: every shipped client would reject valid configs, and it
/// cannot be fixed from the server. `free_tier_enabled` is the literal
/// `true`/`false`.
fn config_payload(
    kid: &str,
    tenant_id: &str,
    product_id: &str,
    issued_at: i64,
    expires_at: i64,
    trial_duration_days: u32,
    free_tier_enabled: bool,
) -> String {
    format!(
        "cfg1|{kid}|{tenant_id}|{product_id}|{issued_at}|{expires_at}|{trial_duration_days}|{}",
        if free_tier_enabled { "true" } else { "false" }
    )
}

/// Verify the Ed25519 signature over a set of server-owned product settings.
///
/// `tenant_id` and `product_id` come from the caller's own configuration, never
/// from the body — that is what makes a config signed for another product fail
/// rather than validate against its own claim.
///
/// Freshness applies to the wire, not to the cache: a response fetched outside
/// its own window is rejected here, while a config already cached stays usable
/// past that window, because expiring it would cut an offline user's trial
/// short for no security gain.
pub fn verify_config(
    fields: &crate::ProductConfigFields,
    tenant_id: &str,
    product_id: &str,
    trusted_keys: &HashMap<String, String>,
    now_seconds: i64,
    skew_seconds: i64,
) -> bool {
    let (issued_at, expires_at, kid, signature) = match (
        fields.issued_at,
        fields.expires_at,
        fields.kid.as_deref(),
        fields.signature.as_deref(),
    ) {
        (Some(i), Some(e), Some(k), Some(s)) if !s.is_empty() => (i, e, k, s),
        // The four fields arrive together or not at all, so a partial set is
        // unsigned rather than malformed.
        _ => return false,
    };

    if now_seconds + skew_seconds < issued_at {
        return false;
    }
    if expires_at + skew_seconds < now_seconds {
        return false;
    }

    // The worker signs a complete pair or sends the response unsigned, so a
    // partial config was never signable and must not be treated as if it were.
    let (days, free_tier) = match (fields.trial_duration_days, fields.free_tier_enabled) {
        (Some(d), Some(f)) => (d, f),
        _ => return false,
    };

    let pub_b64 = match trusted_keys.get(kid) {
        Some(p) => p,
        None => return false,
    };

    (|| -> Option<bool> {
        let pk_bytes: [u8; 32] = b64_decode(pub_b64)?.try_into().ok()?;
        let vk = VerifyingKey::from_bytes(&pk_bytes).ok()?;
        let sig = Signature::from_slice(&b64_decode(signature)?).ok()?;
        let payload = config_payload(
            kid, tenant_id, product_id, issued_at, expires_at, days, free_tier,
        );
        Some(vk.verify(payload.as_bytes(), &sig).is_ok())
    })()
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    // A real round-trip is covered by the conformance gate (tests/conformance.rs).
    #[test]
    fn unknown_kid_short_circuits() {
        let lease = Lease {
            kid: "k9".into(),
            license_key_hash: "a".into(),
            instance_id: "i".into(),
            issued_at: 0,
            expires_at: 100,
            status: "active".into(),
            signature: "x".into(),
            entitlements: vec![],
        };
        let r = verify_lease(&lease, &HashMap::new(), 50, SKEW_SECONDS);
        assert_eq!(
            r,
            VerifyResult {
                kid_known: false,
                signature_valid: false,
                expired: false
            }
        );
    }
    #[test]
    fn expiry_uses_skew() {
        let lease = Lease {
            kid: "k1".into(),
            license_key_hash: "a".into(),
            instance_id: "i".into(),
            issued_at: 0,
            expires_at: 1000,
            status: "active".into(),
            signature: "x".into(),
            entitlements: vec![],
        };
        let keys = HashMap::new(); // kid unknown, but expired is computed regardless
        assert!(!verify_lease(&lease, &keys, 1000 + 200, SKEW_SECONDS).expired);
        assert!(verify_lease(&lease, &keys, 1000 + 400, SKEW_SECONDS).expired);
    }
}
