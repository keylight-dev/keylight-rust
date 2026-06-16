//! The backward clock-rollback guard in `state()`: a valid license resolves to
//! `Licensed`, a clock rolled back past tolerance forces `Invalid`, and the next
//! re-anchor of `last_seen` (what a successful `validate()` does) self-heals it.

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::store::{account, LicenseStore};
use keylight::{Keylight, KeylightConfig, LicenseState};
use std::sync::Arc;

const KID: &str = "k1";

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

struct Noop;
impl Transport for Noop {
    fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> TransportOutcome {
        TransportOutcome::Response(HttpResponse {
            status: 200,
            body: "{}".into(),
            retry_after: None,
        })
    }
    fn get(&self, _: &str, _: &[(String, String)]) -> TransportOutcome {
        TransportOutcome::Response(HttpResponse {
            status: 200,
            body: "{}".into(),
            retry_after: None,
        })
    }
}

fn signed_active_lease(signing: &SigningKey, expires_at: i64) -> String {
    let payload = format!("v3|{KID}|hash|i1|0|{expires_at}|active|pro");
    let sig = signing.sign(payload.as_bytes());
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
    serde_json::json!({
        "kid": KID, "licenseKeyHash": "hash", "instanceId": "i1",
        "issuedAt": 0, "expiresAt": expires_at, "status": "active",
        "signature": sig_b64, "entitlements": ["pro"],
    })
    .to_string()
}

/// Build a client whose store holds a current, signature-valid `active` lease and
/// the given `last_seen` anchor. Returns the shared store handle so a test can
/// re-anchor `last_seen` the way a successful `validate()` would.
fn licensed_client(dir: &str, last_seen: i64) -> (Keylight, Arc<EncryptedFileStore>) {
    let signing = SigningKey::from_bytes(&[7u8; 32]);
    let pub_b64 =
        base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes());

    let d = std::env::temp_dir().join(dir);
    let _ = std::fs::remove_dir_all(&d);
    let store =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    store.set_string(account::LICENSE_KEY, "PRO-KEY").unwrap();
    store.set_string(account::INSTANCE_ID, "i1").unwrap();
    store
        .set_string(
            account::LEASE,
            &signed_active_lease(&signing, now() + 100_000),
        )
        .unwrap();
    store
        .set_string(account::LAST_SEEN, &last_seen.to_string())
        .unwrap();

    let cfg = KeylightConfig::builder("t", "p", "sdk_live_test")
        .trusted_key(KID, pub_b64)
        .build();
    let kl = Keylight::with_parts(cfg, store.clone(), Arc::new(Noop));
    (kl, store)
}

#[test]
fn normal_clock_resolves_licensed() {
    // last_seen ~= now (and a long offline stretch is fine — that's not a rollback).
    let (kl, _store) = licensed_client("kl-clock-normal", now() - 10 * 86400);
    assert_eq!(kl.state(), LicenseState::Licensed);
}

#[test]
fn clock_rolled_back_forces_invalid() {
    // last_seen sits far in the "future" relative to now() -> the clock was rolled
    // back since our last contact -> Invalid, even though the lease itself is valid.
    let (kl, _store) = licensed_client("kl-clock-rollback", now() + 100_000);
    assert_eq!(kl.state(), LicenseState::Invalid);
}

#[test]
fn reanchoring_last_seen_self_heals() {
    // Rolled-back clock -> Invalid.
    let (kl, store) = licensed_client("kl-clock-heal", now() + 100_000);
    assert_eq!(kl.state(), LicenseState::Invalid);

    // A successful validate() re-anchors last_seen to the current time; with the
    // anchor current the guard no longer trips and state() resolves normally.
    store
        .set_string(account::LAST_SEEN, &now().to_string())
        .unwrap();
    assert_eq!(kl.state(), LicenseState::Licensed);
}
