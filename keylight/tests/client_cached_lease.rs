use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::{Keylight, KeylightConfig};
use std::sync::Arc;

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

/// With no stored lease, cached_lease() must return None and has_entitlement() must be false.
#[test]
fn no_lease_means_no_entitlement_and_no_cached_lease() {
    let d = std::env::temp_dir().join("kl-cached-1");
    let _ = std::fs::remove_dir_all(&d);
    let store =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    let cfg = KeylightConfig::builder("t", "p").build();
    let kl = Keylight::with_parts(cfg, store, Arc::new(Noop));
    assert!(kl.cached_lease().is_none());
    assert!(!kl.has_entitlement("pro"));
}
