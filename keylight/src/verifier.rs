use crate::lease::Lease;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey, Verifier as _};
use std::collections::HashMap;

pub const SKEW_SECONDS: i64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyResult {
    pub kid_known: bool,
    pub signature_valid: bool,
    pub expired: bool,
}

/// Decode standard or url-safe base64, tolerating missing padding.
fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let norm: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let norm = norm.replace('-', "+").replace('_', "/");
    let padded = match norm.len() % 4 {
        0 => norm,
        n => format!("{norm}{}", "=".repeat(4 - n)),
    };
    base64::engine::general_purpose::STANDARD.decode(padded.as_bytes()).ok()
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
        None => return VerifyResult { kid_known: false, signature_valid: false, expired },
    };
    let signature_valid = (|| -> Option<bool> {
        let pk_bytes: [u8; 32] = b64_decode(pub_b64)?.try_into().ok()?;
        let vk = VerifyingKey::from_bytes(&pk_bytes).ok()?;
        let sig = Signature::from_slice(&b64_decode(&lease.signature)?).ok()?;
        Some(vk.verify(lease.payload().as_bytes(), &sig).is_ok())
    })().unwrap_or(false);
    VerifyResult { kid_known: true, signature_valid, expired }
}

#[cfg(test)]
mod tests {
    use super::*;
    // A real round-trip is covered by the conformance gate (tests/conformance.rs).
    #[test]
    fn unknown_kid_short_circuits() {
        let lease = Lease {
            kid: "k9".into(), license_key_hash: "a".into(), instance_id: "i".into(),
            issued_at: 0, expires_at: 100, status: "active".into(),
            signature: "x".into(), entitlements: vec![],
        };
        let r = verify_lease(&lease, &HashMap::new(), 50, SKEW_SECONDS);
        assert_eq!(r, VerifyResult { kid_known: false, signature_valid: false, expired: false });
    }
    #[test]
    fn expiry_uses_skew() {
        let lease = Lease {
            kid: "k1".into(), license_key_hash: "a".into(), instance_id: "i".into(),
            issued_at: 0, expires_at: 1000, status: "active".into(),
            signature: "x".into(), entitlements: vec![],
        };
        let keys = HashMap::new(); // kid unknown, but expired is computed regardless
        assert!(!verify_lease(&lease, &keys, 1000 + 200, SKEW_SECONDS).expired);
        assert!(verify_lease(&lease, &keys, 1000 + 400, SKEW_SECONDS).expired);
    }
}
