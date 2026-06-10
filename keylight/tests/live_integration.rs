// Run with: cargo test -p keylight --test live_integration -- --ignored
use keylight::{Keylight, KeylightConfig};
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::store::device::FixedDeviceIdentity;
use std::sync::Arc;

fn live_client(dev: &str) -> Keylight {
    let mut cfg = KeylightConfig::builder("keylight-notes-demo", "notes").key_prefix("NOTES").build();
    let (_, keys) = keylight::keyset::fetch_keyset(&keylight::http::ureq_transport::UreqTransport::default(), &cfg.base_url, &cfg.tenant_id).expect("keyset");
    cfg.trusted_keys.extend(keys);
    let dir = std::env::temp_dir().join(format!("kl-live-{dev}"));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Arc::new(EncryptedFileStore::at_dir(dir, &FixedDeviceIdentity(dev.into())).unwrap());
    Keylight::with_parts(cfg, store, Arc::new(keylight::http::ureq_transport::UreqTransport::default()))
}

#[test] #[ignore]
fn pro_key_activates_with_pro_entitlement() {
    let kl = live_client("ci-pro");
    let r = kl.activate("NOTES-PRO0-0000-0001").unwrap();
    assert!(r.activated, "error: {:?}", r.error);
    assert!(kl.has_entitlement("pro"));
    kl.deactivate().unwrap();
}
#[test] #[ignore]
fn revoked_key_is_rejected() {
    let kl = live_client("ci-revk");
    let r = kl.activate("NOTES-REVK-0000-0002").unwrap();
    assert!(!r.activated);
}
