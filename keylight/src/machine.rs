//! Keyless-heartbeat machine hash: parity with the Swift/other Keylight SDKs.
//!
//! `machine_hash` lets the server dedupe keyless/free-tier devices by a *true*
//! OS/hardware identifier instead of the random per-install fallback id. It is
//! only ever computed from [`crate::store::device::DeviceIdentity::hardware_id`]
//! (never from `stable_id`, which may be the random fallback) — callers must
//! omit the field entirely when no true hardware id is available.

/// `sha256(format!("keylight-keyless-machine-v1|{tenant_id}|{product_id}|{stable_id}"))`,
/// lowercase hex. Must match byte-for-byte across all Keylight SDKs.
pub(crate) fn machine_hash(tenant_id: &str, product_id: &str, stable_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let material = format!("keylight-keyless-machine-v1|{tenant_id}|{product_id}|{stable_id}");
    let digest = Sha256::digest(material.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_hash_matches_cross_sdk_test_vector() {
        // Canonical test value shared across all Keylight SDKs (Swift, Rust, ...).
        assert_eq!(
            machine_hash("testco", "testapp", "hardware-1"),
            "8e8871112f28cabda180ada131d0b4f4f07c72fb47c5d884edbe32812885b22a"
        );
    }
}
