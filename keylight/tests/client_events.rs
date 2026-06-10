use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::{Keylight, KeylightConfig};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct ValidInvalid; // returns valid:false with no lease → state resolves without crypto
impl Transport for ValidInvalid {
    fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> TransportOutcome {
        TransportOutcome::Response(HttpResponse {
            status: 200,
            body: r#"{"valid":false}"#.into(),
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
fn validate_runs_with_event_handler_registered() {
    let d = std::env::temp_dir().join("kl-events-1");
    let _ = std::fs::remove_dir_all(&d);
    let store =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    let cfg = KeylightConfig::builder("t", "p").build();
    let counter = Arc::new(AtomicUsize::new(0));
    let c2 = counter.clone();
    let kl =
        Keylight::with_parts(cfg, store, Arc::new(ValidInvalid)).with_event_handler(move |_ev| {
            c2.fetch_add(1, Ordering::SeqCst);
        });
    // No license stored → validate() returns NoStoredLicense cleanly (handler never fires).
    let err = kl.validate().unwrap_err();
    assert!(matches!(err, keylight::KeylightError::NoStoredLicense));
    assert_eq!(counter.load(Ordering::SeqCst), 0);
}
