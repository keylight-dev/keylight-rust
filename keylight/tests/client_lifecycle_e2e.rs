//! End-to-end lifecycle-event test: drive real Ed25519-signed leases through
//! `validate()` and assert the handler fires the right transition. This guards the
//! full wiring (snapshot prev state -> persist verified lease -> recompute -> fire),
//! which the unit tests on `state::lifecycle_event` alone don't cover.

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::store::{LicenseStore, account};
use keylight::{Keylight, KeylightConfig, LicenseLifecycleEvent};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const KID: &str = "k1";

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Build a signed `v3` lease JSON string matching `Lease::payload()`.
fn signed_lease(signing: &SigningKey, status: &str, expires_at: i64) -> String {
    let ents = ["pro"]; // single entitlement -> csv "pro"
    let payload = format!(
        "v3|{KID}|hash|i1|0|{expires_at}|{status}|{}",
        ents.join(",")
    );
    let sig = signing.sign(payload.as_bytes());
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
    serde_json::json!({
        "kid": KID, "licenseKeyHash": "hash", "instanceId": "i1",
        "issuedAt": 0, "expiresAt": expires_at, "status": status,
        "signature": sig_b64, "entitlements": ents,
    })
    .to_string()
}

/// Transport that returns a queued sequence of response bodies, one per POST.
struct Scripted {
    bodies: Vec<String>,
    idx: AtomicUsize,
}
impl Transport for Scripted {
    fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> TransportOutcome {
        let i = self
            .idx
            .fetch_add(1, Ordering::SeqCst)
            .min(self.bodies.len() - 1);
        TransportOutcome::Response(HttpResponse {
            status: 200,
            body: self.bodies[i].clone(),
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

#[test]
fn validate_fires_cancelled_on_licensed_to_limited() {
    let signing = SigningKey::from_bytes(&[7u8; 32]);
    let pub_b64 =
        base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes());

    let exp = now() + 100_000; // comfortably current under the 300s skew
    let active = format!(
        r#"{{"valid":true,"lease":{}}}"#,
        signed_lease(&signing, "active", exp)
    );
    let fallback = format!(
        r#"{{"valid":false,"lease":{}}}"#,
        signed_lease(&signing, "fallback", exp)
    );

    let dir = std::env::temp_dir().join("kl-lifecycle-e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let store =
        Arc::new(EncryptedFileStore::at_dir(dir, &FixedDeviceIdentity("dev".into())).unwrap());
    // Seed a stored license + instance so validate() proceeds.
    store.set_string(account::LICENSE_KEY, "PRO-KEY").unwrap();
    store.set_string(account::INSTANCE_ID, "i1").unwrap();

    let cfg = KeylightConfig::builder("t", "p", "sdk_live_test")
        .trusted_key(KID, pub_b64)
        .build();
    let transport = Arc::new(Scripted {
        bodies: vec![active, fallback],
        idx: AtomicUsize::new(0),
    });

    let events: Arc<Mutex<Vec<LicenseLifecycleEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let kl = Keylight::with_parts(cfg, store, transport)
        .with_event_handler(move |ev| sink.lock().unwrap().push(ev));

    // Call 1: active lease -> Licensed (transitions from Expired-with-stored-key -> Restored).
    let r1 = kl.validate().unwrap();
    assert!(r1.valid);
    assert!(kl.has_entitlement("pro"));

    // Call 2: server says invalid but hands back a signature-valid fallback lease ->
    // Limited. Licensed -> Limited must fire Cancelled.
    let r2 = kl.validate().unwrap();
    assert!(!r2.valid);

    let got = events.lock().unwrap().clone();
    assert!(
        got.contains(&LicenseLifecycleEvent::Cancelled),
        "expected Cancelled in fired events, got {got:?}"
    );
    assert!(
        got.contains(&LicenseLifecycleEvent::Restored),
        "expected Restored (Expired->Licensed) in fired events, got {got:?}"
    );
}
