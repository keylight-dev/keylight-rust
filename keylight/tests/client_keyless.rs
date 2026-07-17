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

    let body = transport
        .last_body
        .lock()
        .unwrap()
        .clone()
        .expect("a heartbeat should have been posted");
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

    let body = transport
        .last_body
        .lock()
        .unwrap()
        .clone()
        .expect("a heartbeat should have been posted");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        json.get("machine_hash").is_none(),
        "machine_hash must be absent, got body: {body}"
    );
}

use keylight::store::device::DeviceIdentity;
use std::sync::atomic::{AtomicU32, Ordering};

/// Device whose hardware id succeeds only on the first read (models transient
/// ioreg/reg/file failures on later reads).
struct FlakyOnceDevice {
    reads: AtomicU32,
}
impl DeviceIdentity for FlakyOnceDevice {
    fn stable_id(&self) -> String {
        "hardware-1".into()
    }
    fn hardware_id(&self) -> Option<String> {
        if self.reads.fetch_add(1, Ordering::SeqCst) == 0 {
            Some("hardware-1".into())
        } else {
            None
        }
    }
}

/// Transport that fails with a transient error N times, then returns 200,
/// capturing every posted body.
struct FlakyTransport {
    failures: AtomicU32,
    bodies: Mutex<Vec<String>>,
}
impl Transport for FlakyTransport {
    fn post_json(&self, _u: &str, _h: &[(String, String)], body: &str) -> TransportOutcome {
        if self.failures.load(Ordering::SeqCst) > 0 {
            self.failures.fetch_sub(1, Ordering::SeqCst);
            return TransportOutcome::Transient("connection reset".into());
        }
        self.bodies.lock().unwrap().push(body.to_string());
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

fn last_json(t: &CapturingOk) -> serde_json::Value {
    serde_json::from_str(&t.last_body.lock().unwrap().clone().unwrap()).unwrap()
}

#[test]
fn keyless_machine_hash_survives_transient_hardware_id_failure() {
    // First beacon reads the hardware id and caches it; second beacon's fresh
    // read fails but the persisted id keeps machine_hash present and identical.
    let transport = Arc::new(CapturingOk::new());
    let kl = client("kl-keyless-cache", transport.clone()).with_device(Arc::new(FlakyOnceDevice {
        reads: AtomicU32::new(0),
    }));
    kl.report_keyless_state(KeylessState::FreeTier);
    assert_eq!(
        last_json(&transport)
            .get("machine_hash")
            .and_then(|v| v.as_str()),
        Some(CANONICAL_HASH)
    );
    // State change bypasses the 24h debounce so a second beacon is sent.
    kl.report_keyless_state(KeylessState::Trial);
    assert_eq!(
        last_json(&transport)
            .get("machine_hash")
            .and_then(|v| v.as_str()),
        Some(CANONICAL_HASH),
        "cached hardware id must be reused when a fresh read fails"
    );
}

#[test]
fn keyless_beacon_retries_transient_failures_and_persists_debounce_on_200() {
    let transport = Arc::new(FlakyTransport {
        failures: AtomicU32::new(2),
        bodies: Mutex::new(Vec::new()),
    });
    let kl = client_with("kl-keyless-retry", transport.clone())
        .with_device(Arc::new(FixedDeviceIdentity("hardware-1".into())));
    kl.report_keyless_state(KeylessState::FreeTier);
    assert_eq!(
        transport.bodies.lock().unwrap().len(),
        1,
        "beacon should retry through transient failures and land"
    );
    // Debounce persisted on the 200: an immediate same-state beacon is skipped.
    kl.report_keyless_state(KeylessState::FreeTier);
    assert_eq!(transport.bodies.lock().unwrap().len(), 1);
}

fn client_with(dir: &str, transport: Arc<FlakyTransport>) -> Keylight {
    let d = std::env::temp_dir().join(dir);
    let _ = std::fs::remove_dir_all(&d);
    let store =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    let cfg = KeylightConfig::builder("testco", "testapp", "sdk_live_test").build();
    Keylight::with_parts(cfg, store, transport)
}

#[test]
fn activate_and_validate_include_machine_hash_and_deactivate_has_telemetry() {
    let lease_ok = r#"{"activated":true,"valid":true,"instance_id":"i1","license_expires_at":null,"lease":null,"error":null}"#;
    struct AlwaysOk {
        body: String,
        bodies: Mutex<Vec<String>>,
    }
    impl Transport for AlwaysOk {
        fn post_json(&self, _u: &str, _h: &[(String, String)], body: &str) -> TransportOutcome {
            self.bodies.lock().unwrap().push(body.to_string());
            TransportOutcome::Response(HttpResponse {
                status: 200,
                body: self.body.clone(),
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
    let transport = Arc::new(AlwaysOk {
        body: lease_ok.into(),
        bodies: Mutex::new(Vec::new()),
    });
    let d = std::env::temp_dir().join("kl-keyless-actval");
    let _ = std::fs::remove_dir_all(&d);
    let store =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    let cfg = KeylightConfig::builder("testco", "testapp", "sdk_live_test").build();
    let kl = Keylight::with_parts(cfg, store, transport.clone())
        .with_device(Arc::new(FixedDeviceIdentity("hardware-1".into())));

    assert!(kl.activate("TEST-KEY0-0000-0001").unwrap().activated);
    assert!(kl.validate().unwrap().valid);
    kl.deactivate().unwrap();

    let bodies = transport.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 3);
    let activate: serde_json::Value = serde_json::from_str(&bodies[0]).unwrap();
    let validate: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();
    let deactivate: serde_json::Value = serde_json::from_str(&bodies[2]).unwrap();
    assert_eq!(
        activate.get("machine_hash").and_then(|v| v.as_str()),
        Some(CANONICAL_HASH)
    );
    assert_eq!(
        validate.get("machine_hash").and_then(|v| v.as_str()),
        Some(CANONICAL_HASH)
    );
    // Telemetry parity: deactivate now carries the same telemetry fields.
    assert!(deactivate.get("sdk_version").is_some());
    assert!(deactivate.get("platform").is_some());
}

#[test]
fn activate_omits_machine_hash_when_never_readable() {
    struct AlwaysOkSimple(Mutex<Vec<String>>);
    impl Transport for AlwaysOkSimple {
        fn post_json(&self, _u: &str, _h: &[(String, String)], body: &str) -> TransportOutcome {
            self.0.lock().unwrap().push(body.to_string());
            TransportOutcome::Response(HttpResponse {
                status: 200,
                body: r#"{"activated":true,"instance_id":"i1","license_expires_at":null,"lease":null,"error":null}"#.into(),
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
    let transport = Arc::new(AlwaysOkSimple(Mutex::new(Vec::new())));
    let d = std::env::temp_dir().join("kl-keyless-nohw");
    let _ = std::fs::remove_dir_all(&d);
    let store =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    let cfg = KeylightConfig::builder("testco", "testapp", "sdk_live_test").build();
    let kl = Keylight::with_parts(cfg, store, transport.clone())
        .with_device(Arc::new(FixedDeviceIdentity("".into())));
    assert!(kl.activate("TEST-KEY0-0000-0001").unwrap().activated);
    let body: serde_json::Value = serde_json::from_str(&transport.0.lock().unwrap()[0]).unwrap();
    assert!(body.get("machine_hash").is_none());
}
