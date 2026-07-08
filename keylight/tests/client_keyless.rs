//! Keyless heartbeat `machine_hash` field: parity with the Swift/other SDKs.
//! Canonical value: sha256("keylight-keyless-machine-v1|testco|testapp|hardware-1")
//!   = 8e8871112f28cabda180ada131d0b4f4f07c72fb47c5d884edbe32812885b22a

use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::state::KeylessState;
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::{Keylight, KeylightConfig};
use std::sync::{Arc, Mutex};

const CANONICAL_HASH: &str = "8e8871112f28cabda180ada131d0b4f4f07c72fb47c5d884edbe32812885b22a";

/// Transport that returns 200 OK and captures the last posted body for inspection.
struct CapturingOk {
    last_body: Mutex<Option<String>>,
}
impl CapturingOk {
    fn new() -> Self {
        Self {
            last_body: Mutex::new(None),
        }
    }
}
impl Transport for CapturingOk {
    fn post_json(&self, _u: &str, _h: &[(String, String)], body: &str) -> TransportOutcome {
        *self.last_body.lock().unwrap() = Some(body.to_string());
        TransportOutcome::Response(HttpResponse {
            status: 200,
            body: "{}".into(),
            retry_after: None,
        })
    }
    fn get(&self, _u: &str, _h: &[(String, String)]) -> TransportOutcome {
        TransportOutcome::Response(HttpResponse {
            status: 200,
            body: "{}".into(),
            retry_after: None,
        })
    }
}

fn client(dir: &str, transport: Arc<CapturingOk>) -> Keylight {
    let d = std::env::temp_dir().join(dir);
    let _ = std::fs::remove_dir_all(&d);
    let store =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    let cfg = KeylightConfig::builder("testco", "testapp", "sdk_live_test").build();
    Keylight::with_parts(cfg, store, transport)
}

#[test]
fn keyless_heartbeat_includes_machine_hash_when_hardware_id_present() {
    let transport = Arc::new(CapturingOk::new());
    let kl = client("kl-keyless-1", transport.clone())
        .with_device(Arc::new(FixedDeviceIdentity("hardware-1".into())));
    kl.report_keyless_state(KeylessState::FreeTier);

    let body = transport.last_body.lock().unwrap().clone().expect("a heartbeat should have been posted");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        json.get("machine_hash").and_then(|v| v.as_str()),
        Some(CANONICAL_HASH)
    );
}

#[test]
fn keyless_heartbeat_omits_machine_hash_when_no_hardware_id() {
    let transport = Arc::new(CapturingOk::new());
    // Empty FixedDeviceIdentity models "no true hardware id available" — a random
    // per-install fallback must NOT be substituted; machine_hash must be omitted.
    let kl = client("kl-keyless-2", transport.clone())
        .with_device(Arc::new(FixedDeviceIdentity("".into())));
    kl.report_keyless_state(KeylessState::FreeTier);

    let body = transport.last_body.lock().unwrap().clone().expect("a heartbeat should have been posted");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("machine_hash").is_none(),
        "machine_hash must be absent, got body: {body}"
    );
}
