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

/// After deactivate(), LAST_STATE must be cleared so stale state labels don't survive
/// into the next activate→validate cycle.
#[test]
fn deactivate_clears_last_state() {
    use keylight::store::account;
    use keylight::store::LicenseStore;

    let d = std::env::temp_dir().join("kl-cached-2");
    let _ = std::fs::remove_dir_all(&d);
    let store =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    let cfg = KeylightConfig::builder("t", "p").build();
    let store_dyn: Arc<dyn keylight::LicenseStore> =
        Arc::clone(&store) as Arc<dyn keylight::LicenseStore>;
    let kl = Keylight::with_parts(cfg, store_dyn, Arc::new(Noop));

    // Seed a stale LAST_STATE as if a previous activate→validate cycle wrote it.
    store.set_string(account::LAST_STATE, "Active").unwrap();
    assert_eq!(
        store.get_string(account::LAST_STATE).as_deref(),
        Some("Active")
    );

    // deactivate() will 200 (no stored license_key/instance_id so the network call
    // is skipped entirely) then delete all account keys including LAST_STATE.
    let _ = kl.deactivate();
    assert!(
        store.get_string(account::LAST_STATE).is_none(),
        "LAST_STATE must be cleared by deactivate()"
    );
}
