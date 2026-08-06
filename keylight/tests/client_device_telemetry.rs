//! Phase-3 device telemetry: `os_version` and `arch` ride the activate /
//! validate / keyless bodies. `device_class` is NEVER sent — the server only
//! honors it from iOS SDKs and derives desktop classes from the OS token.
//! Deactivate carries nothing new.
use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::{KeylessState, Keylight, KeylightConfig};
use std::sync::{Arc, Mutex};

/// Transport that returns a fixed 200 body and captures every posted body.
struct CapturingOk {
    body: String,
    bodies: Mutex<Vec<String>>,
}
impl CapturingOk {
    fn new(body: &str) -> Self {
        Self {
            body: body.into(),
            bodies: Mutex::new(Vec::new()),
        }
    }
}
impl Transport for CapturingOk {
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

fn client(dir: &str, transport: Arc<CapturingOk>) -> Keylight {
    let d = std::env::temp_dir().join(dir);
    let _ = std::fs::remove_dir_all(&d);
    let store =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    let cfg = KeylightConfig::builder("testco", "testapp", "sdk_live_test").build();
    Keylight::with_parts(cfg, store, transport)
        .with_device(Arc::new(FixedDeviceIdentity("hardware-1".into())))
}

/// The canonical arch spelling the server allow-lists for this build target,
/// or None on targets outside the vocabulary (which must OMIT the field).
fn expected_arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "aarch64" => Some("arm64"),
        "x86_64" => Some("x86_64"),
        _ => None,
    }
}

fn is_dotted_numeric(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn assert_device_fields(body: &serde_json::Value, route: &str) {
    // arch: canonical spelling, or absent off the allow-list.
    match expected_arch() {
        Some(a) => assert_eq!(
            body.get("arch").and_then(|v| v.as_str()),
            Some(a),
            "{route}: arch must be the canonical spelling"
        ),
        None => assert!(
            body.get("arch").is_none(),
            "{route}: arch must be omitted on unlisted targets"
        ),
    }
    // os_version: when present it must be dotted-numeric — the server nulls
    // anything else, and we never ship junk bytes. On the three desktop OSes
    // it should actually be present.
    if let Some(v) = body.get("os_version") {
        let s = v.as_str().expect("os_version must be a string");
        assert!(
            is_dotted_numeric(s),
            "{route}: os_version {s:?} is not dotted-numeric"
        );
    }
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    assert!(
        body.get("os_version").is_some(),
        "{route}: os_version should be readable on this OS"
    );
    // device_class: this SDK must NEVER send it (iOS-only field server-side).
    assert!(
        body.get("device_class").is_none(),
        "{route}: device_class must never be sent by this SDK"
    );
}

#[test]
fn activate_validate_and_keyless_carry_device_fields_but_deactivate_does_not() {
    let ok = r#"{"activated":true,"valid":true,"instance_id":"i1","license_expires_at":null,"lease":null,"error":null}"#;
    let transport = Arc::new(CapturingOk::new(ok));
    let kl = client("kl-device-telemetry", transport.clone());

    assert!(kl.activate("TEST-KEY0-0000-0001").unwrap().activated);
    assert!(kl.validate().unwrap().valid);
    kl.report_keyless_state(KeylessState::FreeTier);
    kl.deactivate().unwrap();

    let bodies = transport.bodies.lock().unwrap();
    assert_eq!(bodies.len(), 4);
    let parsed: Vec<serde_json::Value> = bodies
        .iter()
        .map(|b| serde_json::from_str(b).unwrap())
        .collect();

    assert_device_fields(&parsed[0], "activate");
    assert_device_fields(&parsed[1], "validate");
    assert_device_fields(&parsed[2], "keyless");

    // Deactivate keeps its existing telemetry but gains nothing new.
    let deactivate = &parsed[3];
    assert!(deactivate.get("sdk_version").is_some());
    assert!(deactivate.get("platform").is_some());
    assert!(
        deactivate.get("os_version").is_none(),
        "deactivate must not send os_version"
    );
    assert!(
        deactivate.get("arch").is_none(),
        "deactivate must not send arch"
    );
    assert!(deactivate.get("device_class").is_none());
}
