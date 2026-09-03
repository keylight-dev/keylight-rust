//! The core crate's own keyless heartbeat.
//!
//! `report_keyless_state` has never had a cadence in the crate: the Tauri plugin
//! owns one, so a Tauri app is covered, but a resident non-Tauri host — a
//! daemon, a service, a desktop app built without the plugin — beacons once at
//! startup and then looks dead to the dashboard for as long as it runs.
//!
//! The crate cannot simply spawn a thread from `&self`: `Keylight::new` returns
//! a value, and a thread that outlives the borrow needs shared ownership. So the
//! cadence is an explicit `Arc<Self>` method returning an RAII handle — the
//! caller keeps ownership, the thread holds a `Weak` so it can never keep the
//! client alive, and dropping the handle stops and joins it.
use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::state::{KeylessState, LicenseState};
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::store::{LicenseStore, account};
use keylight::{Keylight, KeylightConfig};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Counts every POST to `/keyless` and records the states reported, in order.
/// Locked because the heartbeat thread writes while the test thread reads.
struct CountingOk {
    keyless: AtomicUsize,
    states: Mutex<Vec<String>>,
}
impl CountingOk {
    fn new() -> Self {
        Self {
            keyless: AtomicUsize::new(0),
            states: Mutex::new(Vec::new()),
        }
    }
    fn count(&self) -> usize {
        self.keyless.load(Ordering::SeqCst)
    }
    fn states(&self) -> Vec<String> {
        self.states.lock().unwrap().clone()
    }
}
impl Transport for CountingOk {
    fn post_json(&self, url: &str, _h: &[(String, String)], body: &str) -> TransportOutcome {
        if url.ends_with("/keyless") {
            self.keyless.fetch_add(1, Ordering::SeqCst);
            // Cheap extraction — the body is a flat JSON object.
            if let Some(i) = body.find("\"state\":\"") {
                let rest = &body[i + 9..];
                if let Some(j) = rest.find('"') {
                    self.states.lock().unwrap().push(rest[..j].to_string());
                }
            }
        }
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

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// A keyless client in an active trial, plus its store so a test can move the
/// trial window and make `state()` resolve differently on the next tick.
fn trial_client(dir: &str, transport: Arc<CountingOk>) -> (Arc<Keylight>, Arc<EncryptedFileStore>) {
    let d = std::env::temp_dir().join(dir);
    let _ = std::fs::remove_dir_all(&d);
    let store =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    store
        .set_string(account::TRIAL_START, &now().to_string())
        .unwrap();
    // Free tier on, so an expired trial lands on FreeTier rather than Invalid.
    // Invalid is a denial and correctly beacons nothing, which would make the
    // transition below unobservable.
    let cfg = KeylightConfig::builder("testco", "testapp", "sdk_live_test")
        .trial_duration_days(14)
        .free_tier_enabled(true)
        .build();
    (
        Arc::new(Keylight::with_parts(cfg, store.clone(), transport)),
        store,
    )
}

/// Poll rather than sleep a fixed span: no flake on a loaded machine.
fn wait_for(f: impl Fn() -> bool) -> bool {
    for _ in 0..300 {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    f()
}

#[test]
fn keyless_state_for_maps_only_the_keyless_states() {
    assert_eq!(
        keylight::keyless_state_for(&LicenseState::Trial { days_left: 3 }),
        Some(KeylessState::Trial)
    );
    assert_eq!(
        keylight::keyless_state_for(&LicenseState::FreeTier),
        Some(KeylessState::FreeTier)
    );
    assert_eq!(
        keylight::keyless_state_for(&LicenseState::Expired),
        Some(KeylessState::Expired)
    );
    // A licensed device reports liveness through /validate, not the beacon.
    assert_eq!(keylight::keyless_state_for(&LicenseState::Licensed), None);
    assert_eq!(keylight::keyless_state_for(&LicenseState::Limited), None);
    // Invalid is a denial, not a device to count.
    assert_eq!(keylight::keyless_state_for(&LicenseState::Invalid), None);
}

/// The 24h debounce is real and the test clock cannot be moved, so liveness is
/// proven the other way the debounce allows: a state *change* always sends. Two
/// beacons from two different states means the thread is still running and
/// still re-reading state, which is the property that was missing.
#[test]
fn heartbeat_keeps_reporting_while_the_app_stays_open() {
    let transport = Arc::new(CountingOk::new());
    let (kl, store) = trial_client("kl-hb-1", transport.clone());

    let _hb = kl.start_keyless_heartbeat(Duration::from_millis(10));
    assert!(
        wait_for(|| transport.count() >= 1),
        "first beacon never sent"
    );

    // The trial lapses into the free tier while the app keeps running. Nothing
    // calls into the SDK.
    store
        .set_string(account::TRIAL_START, &(now() - 100 * 86400).to_string())
        .unwrap();

    assert!(
        wait_for(|| transport.count() >= 2),
        "the heartbeat stopped after one tick — got {} beacon(s)",
        transport.count()
    );
    assert_eq!(transport.states(), vec!["trial", "free_tier"]);
}

#[test]
fn dropping_the_handle_stops_the_heartbeat() {
    let transport = Arc::new(CountingOk::new());
    let (kl, store) = trial_client("kl-hb-2", transport.clone());

    let hb = kl.start_keyless_heartbeat(Duration::from_millis(10));
    assert!(wait_for(|| transport.count() >= 1));

    drop(hb);
    let after_stop = transport.count();
    store
        .set_string(account::TRIAL_START, &(now() - 100 * 86400).to_string())
        .unwrap();
    std::thread::sleep(Duration::from_millis(150));

    assert_eq!(
        transport.count(),
        after_stop,
        "a dropped handle must stop the thread"
    );
}

/// The thread holds a Weak, so the client's lifetime is the caller's business.
/// Dropping the last Arc must not be blocked by, or outlive, the heartbeat.
#[test]
fn the_heartbeat_never_keeps_the_client_alive() {
    let transport = Arc::new(CountingOk::new());
    let (kl, _store) = trial_client("kl-hb-3", transport.clone());

    let hb = kl.start_keyless_heartbeat(Duration::from_millis(10));
    assert!(wait_for(|| transport.count() >= 1));

    assert_eq!(
        Arc::strong_count(&kl),
        1,
        "the thread must not hold a strong ref"
    );
    drop(kl);

    // The handle outliving the client is safe: the tick upgrades a Weak and
    // exits when it fails. Dropping it must still return, not hang on a join.
    drop(hb);
}

#[test]
fn a_zero_interval_is_refused_rather_than_spinning() {
    let transport = Arc::new(CountingOk::new());
    let (kl, _store) = trial_client("kl-hb-4", transport.clone());

    let _hb = kl.start_keyless_heartbeat(Duration::from_millis(0));
    std::thread::sleep(Duration::from_millis(80));

    assert_eq!(transport.count(), 0, "a zero interval must not spin-beacon");
}
