use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::{Keylight, KeylightConfig};
use std::sync::Arc;

struct MockOk(String);
impl Transport for MockOk {
    fn post_json(&self, _u: &str, _h: &[(String, String)], _b: &str) -> TransportOutcome {
        TransportOutcome::Response(HttpResponse {
            status: 200,
            body: self.0.clone(),
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

#[test]
fn activate_rejects_unverifiable_lease() {
    // A lease signed by an unknown kid must fail verification → LeaseVerificationFailed.
    let body = r#"{"activated":true,"instance_id":"i1","license_expires_at":null,"lease":{"kid":"k9","licenseKeyHash":"a","instanceId":"i1","issuedAt":0,"expiresAt":9999999999,"status":"active","signature":"AA","entitlements":["pro"]}}"#;
    let dir = std::env::temp_dir().join("kl-client-test-1");
    let _ = std::fs::remove_dir_all(&dir);
    let store =
        Arc::new(EncryptedFileStore::at_dir(dir, &FixedDeviceIdentity("d".into())).unwrap());
    let cfg = KeylightConfig::builder("keylight-notes-demo", "notes").build(); // no trusted keys
    let kl = Keylight::with_parts(cfg, store, Arc::new(MockOk(body.into())));
    let err = kl.activate("NOTES-PRO0-0000-0001").unwrap_err();
    assert!(matches!(
        err,
        keylight::KeylightError::LeaseVerificationFailed
    ));
    // Verify-before-write invariant: a rejected lease must leave the store empty.
    assert!(!kl.has_stored_license());
    assert!(kl.cached_license_key().is_none());
}

#[test]
fn activate_invalid_format_returns_error_not_request() {
    let dir = std::env::temp_dir().join("kl-client-test-2");
    let _ = std::fs::remove_dir_all(&dir);
    let store =
        Arc::new(EncryptedFileStore::at_dir(dir, &FixedDeviceIdentity("d".into())).unwrap());
    let cfg = KeylightConfig::builder("t", "p")
        .key_prefix("NOTES")
        .build();
    let kl = Keylight::with_parts(cfg, store, Arc::new(MockOk("{}".into())));
    let r = kl.activate("BADKEY").unwrap();
    assert!(!r.activated);
    assert_eq!(r.error.as_deref(), Some("Invalid license key format"));
}
