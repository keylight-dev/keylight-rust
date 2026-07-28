//! `active_revalidate()` — the prompt, active-use revalidation primitive (Swift
//! parity with `LicenseManager.activeRevalidate()`).
//!
//! Contract, in priority order:
//! 1. No-op when no license key is stored (no network call at all).
//! 2. Debounced at 60s, held **in memory** — the window must not survive a
//!    process restart (a fresh client over the same store may call again).
//! 3. Forces a server validate, bypassing the debounce/stale/near-expiry gates
//!    that `refresh_if_needed` applies.
//! 4. A definitive rejection (`valid:false`, e.g. a dashboard revoke's HTTP 422)
//!    downgrades immediately.
//! 5. A transient/network failure leaves state untouched — never downgrade a
//!    live session on a blip.

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::store::{LicenseStore, account};
use keylight::{Keylight, KeylightConfig, LicenseState};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const KID: &str = "k1";

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// Signed `v3` lease JSON (camelCase wire shape) with the given status/expiry.
fn lease_json(signing: &SigningKey, status: &str, expires_at: i64) -> String {
    let payload = format!("v3|{KID}|hash|i1|0|{expires_at}|{status}|pro");
    let sig = signing.sign(payload.as_bytes());
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
    serde_json::json!({
        "kid": KID, "licenseKeyHash": "hash", "instanceId": "i1",
        "issuedAt": 0, "expiresAt": expires_at, "status": status,
        "signature": sig_b64, "entitlements": ["pro"],
    })
    .to_string()
}

fn config_trusting(signing: &SigningKey) -> KeylightConfig {
    let pub_b64 =
        base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes());
    KeylightConfig::builder("t", "p", "sdk_live_test")
        .trusted_key(KID, pub_b64)
        .build()
}

fn empty_store(dir: &str) -> Arc<EncryptedFileStore> {
    let d = std::env::temp_dir().join(dir);
    let _ = std::fs::remove_dir_all(&d);
    Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap())
}

/// A store holding a signature-valid, currently-active lease that was validated
/// online just now — i.e. exactly the situation `refresh_if_needed` would skip.
fn store_with_active_lease(dir: &str, signing: &SigningKey) -> Arc<EncryptedFileStore> {
    let store = empty_store(dir);
    store.set_string(account::LICENSE_KEY, "PRO-KEY").unwrap();
    store.set_string(account::INSTANCE_ID, "i1").unwrap();
    store
        .set_string(
            account::LEASE,
            &lease_json(signing, "active", now() + 100_000),
        )
        .unwrap();
    store
        .set_string(account::LAST_SEEN, &now().to_string())
        .unwrap();
    store
        .set_string(account::LAST_VALIDATED_ONLINE, &now().to_string())
        .unwrap();
    store
}

/// Fixed status + body, counting every POST so tests can assert call counts.
struct Counting {
    status: u16,
    body: String,
    calls: AtomicUsize,
}
impl Counting {
    fn new(status: u16, body: &str) -> Arc<Self> {
        Arc::new(Self {
            status,
            body: body.into(),
            calls: AtomicUsize::new(0),
        })
    }
    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl Transport for Counting {
    fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> TransportOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        TransportOutcome::Response(HttpResponse {
            status: self.status,
            body: self.body.clone(),
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

/// Transport-level failure with no HTTP response at all (offline / DNS), counted.
struct CountingDown {
    calls: AtomicUsize,
}
impl CountingDown {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
        })
    }
    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl Transport for CountingDown {
    fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> TransportOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        TransportOutcome::Terminal("offline".into())
    }
    fn get(&self, _: &str, _: &[(String, String)]) -> TransportOutcome {
        TransportOutcome::Terminal("offline".into())
    }
}

/// (1) No stored license key → no-op: no network call, nothing returned.
#[test]
fn no_stored_license_is_a_noop() {
    let signing = signing_key();
    let store = empty_store("kl-active-noop");
    let transport = Counting::new(200, r#"{"valid":true}"#);
    let kl = Keylight::with_parts(config_trusting(&signing), store, transport.clone());

    assert!(
        kl.active_revalidate().is_none(),
        "no stored license must return None"
    );
    assert_eq!(
        transport.count(),
        0,
        "no stored license must not hit the network"
    );
}

/// (2) + (3) The first call forces a validate even though `refresh_if_needed`
/// would skip (validated online seconds ago), and a second call inside the 60s
/// window is suppressed.
#[test]
fn forces_a_validate_then_debounces_the_second_call() {
    let signing = signing_key();
    let store = store_with_active_lease("kl-active-debounce", &signing);
    let transport = Counting::new(200, r#"{"valid":true}"#);
    let kl = Keylight::with_parts(config_trusting(&signing), store, transport.clone());

    // Precondition: the staleness gate would skip this one.
    assert!(
        kl.refresh_if_needed().unwrap().is_none(),
        "precondition: refresh_if_needed must skip a just-validated license"
    );
    assert_eq!(transport.count(), 0, "precondition: no call yet");

    let first = kl.active_revalidate();
    assert!(
        first.is_some_and(|r| r.valid),
        "active_revalidate must bypass the staleness gate and validate"
    );
    assert_eq!(transport.count(), 1);

    assert!(
        kl.active_revalidate().is_none(),
        "a second call inside the 60s window must be suppressed"
    );
    assert_eq!(
        transport.count(),
        1,
        "the debounced call must not hit the network"
    );
}

/// (2) The debounce lives in memory only: a fresh client over the same store
/// (i.e. a process restart) must be allowed to validate again immediately.
#[test]
fn debounce_is_in_memory_and_does_not_survive_a_restart() {
    let signing = signing_key();
    let store = store_with_active_lease("kl-active-restart", &signing);
    let transport = Counting::new(200, r#"{"valid":true}"#);

    let first = Keylight::with_parts(config_trusting(&signing), store.clone(), transport.clone());
    assert!(first.active_revalidate().is_some());
    assert!(first.active_revalidate().is_none(), "debounced in-process");
    assert_eq!(transport.count(), 1);
    drop(first);

    let restarted = Keylight::with_parts(config_trusting(&signing), store, transport.clone());
    assert!(
        restarted.active_revalidate().is_some(),
        "the debounce must not be persisted across a restart"
    );
    assert_eq!(transport.count(), 2);
}

/// (4) A dashboard revoke (HTTP 422, `{"valid":false,...}`) downgrades right
/// away — no relaunch needed.
#[test]
fn revoke_downgrades_immediately() {
    let signing = signing_key();
    let store = store_with_active_lease("kl-active-revoke", &signing);
    let transport = Counting::new(
        422,
        r#"{"valid":false,"reason":"revoked","error":"License revoked"}"#,
    );
    let kl = Keylight::with_parts(config_trusting(&signing), store.clone(), transport.clone());

    assert_eq!(
        kl.state(),
        LicenseState::Licensed,
        "precondition: cached lease starts out valid"
    );

    let result = kl
        .active_revalidate()
        .expect("a 422 rejection is an outcome");
    assert!(!result.valid, "the revoke must be reported as invalid");
    assert_ne!(
        kl.state(),
        LicenseState::Licensed,
        "a revoke must downgrade the live session immediately"
    );
    assert!(
        store.get_string(account::LEASE).is_none(),
        "the stale 'active' lease must be cleared"
    );
}

/// (5) The safety property: a transport failure must leave the live session
/// exactly as it was.
#[test]
fn transient_failure_does_not_downgrade() {
    let signing = signing_key();
    let store = store_with_active_lease("kl-active-transient", &signing);
    let lease_before = store.get_string(account::LEASE);
    let transport = CountingDown::new();
    let kl = Keylight::with_parts(config_trusting(&signing), store.clone(), transport.clone());

    assert!(
        kl.active_revalidate().is_none(),
        "a transient failure yields no outcome"
    );
    assert!(transport.count() >= 1, "it did try");
    assert_eq!(
        kl.state(),
        LicenseState::Licensed,
        "a network blip must never downgrade a live session"
    );
    assert_eq!(
        store.get_string(account::LEASE),
        lease_before,
        "a transient failure must not mutate the cached lease"
    );
}
