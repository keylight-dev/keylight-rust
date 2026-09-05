//! Trial length and free tier are settings the **server** owns; the values on
//! `KeylightConfig` are only a seed for an install that has never reached the
//! server.
//!
//! Resolution order is server value → local seed → 0. These are the tests that
//! caught real bugs in the C++ port rather than the ones that restate the
//! implementation — see `keylight-cpp`'s
//! `docs/superpowers/specs/2026-09-05-trial-parity-handoff.md`.

use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::store::{LicenseStore, account};
use keylight::{Keylight, KeylightConfig, LicenseState, TrialStatus};
use std::sync::{Arc, Mutex};

/// Transport that answers every call with a fixed body and records the paths it
/// was asked for, so a test can assert what the client did *not* call.
struct Scripted {
    body: String,
    status: u16,
    paths: Mutex<Vec<String>>,
}

impl Scripted {
    fn new(body: &str) -> Arc<Self> {
        Arc::new(Self {
            body: body.into(),
            status: 200,
            paths: Mutex::new(Vec::new()),
        })
    }
    fn failing(status: u16) -> Arc<Self> {
        Arc::new(Self {
            body: r#"{"error":"nope"}"#.into(),
            status,
            paths: Mutex::new(Vec::new()),
        })
    }
    fn respond(&self, url: &str) -> TransportOutcome {
        self.paths.lock().unwrap().push(url.to_string());
        TransportOutcome::Response(HttpResponse {
            status: self.status,
            body: self.body.clone(),
            retry_after: None,
        })
    }
    fn saw_path_ending(&self, suffix: &str) -> bool {
        self.paths
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.ends_with(suffix))
    }
}

impl Transport for Scripted {
    fn post_json(&self, url: &str, _: &[(String, String)], _: &str) -> TransportOutcome {
        self.respond(url)
    }
    fn get(&self, url: &str, _: &[(String, String)]) -> TransportOutcome {
        self.respond(url)
    }
}

struct Harness {
    client: Keylight,
    store: Arc<dyn LicenseStore>,
}

fn harness(
    dir: &str,
    seed_days: u32,
    seed_free_tier: bool,
    transport: Arc<dyn Transport>,
) -> Harness {
    let d = std::env::temp_dir().join(dir);
    let _ = std::fs::remove_dir_all(&d);
    let store: Arc<dyn LicenseStore> =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    let cfg = KeylightConfig::builder("t", "p", "sdk_live_test")
        .trial_duration_days(seed_days)
        .free_tier_enabled(seed_free_tier)
        .build();
    Harness {
        client: Keylight::with_parts(cfg, Arc::clone(&store), transport),
        store,
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn seed_trial_start(store: &Arc<dyn LicenseStore>, days_ago: i64) {
    store
        .set_string(
            account::TRIAL_START,
            &(now() - days_ago * 86400).to_string(),
        )
        .unwrap();
}

// ---------------------------------------------------------------- resolution

/// The headline case: a tenant turns a trial on in the dashboard for a build
/// that shipped with none.
#[test]
fn server_duration_grants_a_trial_when_the_seed_is_zero() {
    let h = harness(
        "kl-cfg-1",
        0,
        false,
        Scripted::new(r#"{"trial_duration_days":14}"#),
    );
    h.client.fetch_config();
    seed_trial_start(&h.store, 3);

    assert_eq!(h.client.effective_trial_duration_days(), 14);
    assert_eq!(
        h.client.check_trial(),
        TrialStatus::Active { days_left: 11 }
    );
}

/// The reverse direction, and the one a tenant actually notices: trials switched
/// off server-side must not be resurrected by the compiled-in seed.
#[test]
fn server_zero_turns_off_a_seed_enabled_trial() {
    let h = harness(
        "kl-cfg-2",
        14,
        false,
        Scripted::new(r#"{"trial_duration_days":0}"#),
    );
    h.client.fetch_config();
    seed_trial_start(&h.store, 1);

    assert_eq!(h.client.effective_trial_duration_days(), 0);
    assert_eq!(h.client.check_trial(), TrialStatus::Expired);
}

/// An absent config falls through to the seed, never to zero. Taking the usual
/// "0 means the field was missing" shortcut would leave every pre-config install
/// with no trial.
#[test]
fn absent_config_falls_through_to_the_seed_not_to_zero() {
    let h = harness("kl-cfg-3", 14, true, Scripted::new("{}"));

    assert_eq!(h.client.effective_trial_duration_days(), 14);
    assert!(h.client.effective_free_tier_enabled());
}

// -------------------------------------------------------------------- stamp

/// The bug that made a dashboard-set trial do nothing.
///
/// `start_trial()` returning early at a zero duration left no start timestamp
/// for a later-arriving duration to measure, so the user never got the trial
/// their tenant enabled.
#[test]
fn start_trial_stamps_even_at_a_zero_duration() {
    let h = harness(
        "kl-cfg-4",
        0,
        false,
        Scripted::new(r#"{"trial_duration_days":14}"#),
    );
    h.client.start_trial().unwrap();

    assert!(
        h.store.get_string(account::TRIAL_START).is_some(),
        "the clock must be stamped even when no trial is on offer"
    );
    assert_eq!(
        h.client.check_trial(),
        TrialStatus::Expired,
        "the stamp alone grants nothing"
    );

    h.client.fetch_config();
    assert_eq!(
        h.client.check_trial(),
        TrialStatus::Active { days_left: 14 },
        "the trial runs from first launch, not from when the config landed"
    );
}

/// The anonymous instance id is minted with the stamp, or an install that
/// started offline can never be attributed when it converts.
#[test]
fn start_trial_mints_the_instance_id_even_at_a_zero_duration() {
    let h = harness("kl-cfg-5", 0, false, Scripted::new("{}"));
    h.client.start_trial().unwrap();

    assert!(h.store.get_string(account::FREE_TIER_INSTANCE_ID).is_some());
}

/// Otherwise the trial is farmable by reinstalling.
#[test]
fn an_old_stamp_is_honoured_never_restarted() {
    let h = harness(
        "kl-cfg-6",
        0,
        false,
        Scripted::new(r#"{"trial_duration_days":14}"#),
    );
    seed_trial_start(&h.store, 60);
    h.client.fetch_config();

    // Assert the duration actually landed first, or this test passes for the
    // wrong reason: a seed of 0 also resolves to Expired.
    assert_eq!(h.client.effective_trial_duration_days(), 14);
    assert_eq!(h.client.check_trial(), TrialStatus::Expired);
}

/// `start_trial()` never overwrites an existing stamp, even when called again.
#[test]
fn start_trial_does_not_restart_an_existing_stamp() {
    let h = harness("kl-cfg-7", 14, false, Scripted::new("{}"));
    seed_trial_start(&h.store, 10);
    let before = h.store.get_string(account::TRIAL_START);

    h.client.start_trial().unwrap();

    assert_eq!(h.store.get_string(account::TRIAL_START), before);
    assert_eq!(h.client.check_trial(), TrialStatus::Active { days_left: 4 });
}

// -------------------------------------------------------------- persistence

/// A relaunch reads the last known server settings, not the seed.
#[test]
fn a_server_zero_survives_a_relaunch_as_zero() {
    let h = harness(
        "kl-cfg-8",
        14,
        false,
        Scripted::new(r#"{"trial_duration_days":0}"#),
    );
    h.client.fetch_config();

    let cfg = KeylightConfig::builder("t", "p", "sdk_live_test")
        .trial_duration_days(14)
        .build();
    let relaunched = Keylight::with_parts(cfg, Arc::clone(&h.store), Scripted::new("{}"));

    assert_eq!(
        relaunched.effective_trial_duration_days(),
        0,
        "a cached zero is a real setting and must survive the process"
    );
}

#[test]
fn a_server_free_tier_false_survives_against_a_seed_of_true() {
    let h = harness(
        "kl-cfg-9",
        14,
        true,
        Scripted::new(r#"{"free_tier_enabled":false}"#),
    );
    h.client.fetch_config();

    assert!(!h.client.effective_free_tier_enabled());
    assert_ne!(
        h.client.state(),
        LicenseState::FreeTier,
        "a server-disabled free tier must not resolve as FreeTier"
    );
}

/// An older worker sends neither field. That must not wipe what this install
/// already learned.
#[test]
fn a_response_with_no_config_fields_leaves_the_cache_alone() {
    let h = harness(
        "kl-cfg-10",
        30,
        false,
        Scripted::new(r#"{"trial_duration_days":7,"free_tier_enabled":true}"#),
    );
    h.client.fetch_config();

    let empty = Keylight::with_parts(
        KeylightConfig::builder("t", "p", "sdk_live_test")
            .trial_duration_days(30)
            .build(),
        Arc::clone(&h.store),
        Scripted::new("{}"),
    );
    empty.fetch_config();

    assert_eq!(empty.effective_trial_duration_days(), 7);
    assert!(empty.effective_free_tier_enabled());
}

/// Each field merges on its own — a response carrying only one must not blank
/// the other.
#[test]
fn a_partial_response_merges_rather_than_replaces() {
    let h = harness(
        "kl-cfg-11",
        30,
        false,
        Scripted::new(r#"{"trial_duration_days":7,"free_tier_enabled":true}"#),
    );
    h.client.fetch_config();

    let partial = Keylight::with_parts(
        KeylightConfig::builder("t", "p", "sdk_live_test")
            .trial_duration_days(30)
            .build(),
        Arc::clone(&h.store),
        Scripted::new(r#"{"trial_duration_days":21}"#),
    );
    partial.fetch_config();

    assert_eq!(partial.effective_trial_duration_days(), 21);
    assert!(
        partial.effective_free_tier_enabled(),
        "the untouched field keeps its cached value"
    );
}

/// A refresh that cannot reach the server keeps the last known settings rather
/// than falling back to the seed.
#[test]
fn a_failed_fetch_keeps_the_cached_value() {
    let h = harness(
        "kl-cfg-12",
        30,
        false,
        Scripted::new(r#"{"trial_duration_days":7}"#),
    );
    h.client.fetch_config();

    let offline = Keylight::with_parts(
        KeylightConfig::builder("t", "p", "sdk_live_test")
            .trial_duration_days(30)
            .build(),
        Arc::clone(&h.store),
        Scripted::failing(500),
    );
    offline.fetch_config();

    assert_eq!(offline.effective_trial_duration_days(), 7);
}

// --------------------------------------------------------------------- wire

/// The settings ride on calls the SDK already makes — `validate` covers every
/// licensed install.
#[test]
fn validate_carries_the_config_and_it_is_absorbed() {
    let transport =
        Scripted::new(r#"{"valid":true,"trial_duration_days":21,"free_tier_enabled":true}"#);
    let h = harness(
        "kl-cfg-13",
        30,
        false,
        Arc::clone(&transport) as Arc<dyn Transport>,
    );
    h.store
        .set_string(account::LICENSE_KEY, "TEST-KEY")
        .unwrap();
    h.store.set_string(account::INSTANCE_ID, "inst-1").unwrap();

    h.client.validate().unwrap();

    assert_eq!(h.client.effective_trial_duration_days(), 21);
    assert!(h.client.effective_free_tier_enabled());
}

/// The keyless beacon is the only route that reaches an unlicensed install.
#[test]
fn the_keyless_beacon_response_carries_the_config() {
    let transport = Scripted::new(r#"{"trial_duration_days":7}"#);
    let h = harness(
        "kl-cfg-14",
        30,
        false,
        Arc::clone(&transport) as Arc<dyn Transport>,
    );

    h.client.report_keyless_state(keylight::KeylessState::Trial);

    assert_eq!(h.client.effective_trial_duration_days(), 7);
}

/// Section 2 of the handoff: do not add a config fetch to the launch path. It
/// would cost a network round-trip per client construction.
#[test]
fn state_resolution_does_not_fetch_config() {
    let transport = Scripted::new(r#"{"trial_duration_days":7}"#);
    let h = harness(
        "kl-cfg-15",
        14,
        false,
        Arc::clone(&transport) as Arc<dyn Transport>,
    );

    h.client.start_trial().unwrap();
    let _ = h.client.state();
    let _ = h.client.check_trial();

    assert!(
        !transport.saw_path_ending("/config"),
        "resolving state must not hit /config — the settings ride on calls already being made"
    );
}
