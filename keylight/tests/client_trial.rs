use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::store::device::FixedDeviceIdentity;
use keylight::{Keylight, KeylightConfig, TrialStatus, LicenseState};
use keylight::http::{Transport, TransportOutcome, HttpResponse};
use std::sync::Arc;

struct Noop;
impl Transport for Noop {
    fn post_json(&self, _:&str,_:&[(String,String)],_:&str)->TransportOutcome { TransportOutcome::Response(HttpResponse{status:200,body:"{}".into(),retry_after:None}) }
    fn get(&self,_:&str,_:&[(String,String)])->TransportOutcome { TransportOutcome::Response(HttpResponse{status:200,body:"{}".into(),retry_after:None}) }
}

fn client(dir: &str, free_tier: bool) -> Keylight {
    let d = std::env::temp_dir().join(dir);
    let _ = std::fs::remove_dir_all(&d);
    let store = Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    let cfg = KeylightConfig::builder("t","p").trial_duration_days(14).free_tier_enabled(free_tier).build();
    Keylight::with_parts(cfg, store, Arc::new(Noop))
}

#[test]
fn trial_lifecycle() {
    let kl = client("kl-trial-1", false);
    assert_eq!(kl.check_trial(), TrialStatus::NotStarted);
    kl.start_trial().unwrap();
    assert!(matches!(kl.check_trial(), TrialStatus::Active { .. }));
    assert!(matches!(kl.state(), LicenseState::Trial { .. }));
}

#[test]
fn no_trial_free_tier_state() {
    let kl = client("kl-trial-2", true);
    assert_eq!(kl.state(), LicenseState::FreeTier);
}
