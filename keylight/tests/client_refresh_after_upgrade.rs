//! `refresh_after_upgrade()` — brief poll-revalidation after a customer
//! completes an upgrade (Swift parity with
//! `LicenseManager.refreshAfterUpgrade(timeout:pollInterval:)`), covering the
//! lag between checkout completing and the payment webhook reaching Keylight.
//!
//! Contract:
//! 1. No stored license → `false`, and **zero** network calls.
//! 2. Re-validates every `poll_interval` (clamped to a 100ms floor) up to
//!    `timeout`.
//! 3. Returns `true` as soon as either the entitlement *set* (order-independent)
//!    or the resolved `state()` differs from what it was when called.
//! 4. A definitive rejection mid-poll (e.g. a downgrade landing) drives
//!    `state()` away from `Licensed` — which is itself a state change, so it
//!    also returns `true`.
//! 5. If the webhook never lands (server keeps echoing the same entitlements
//!    and state), returns `false` once `timeout` elapses, having polled more
//!    than once.
//! 6. A transient/network error during a poll attempt is not treated as a
//!    change (mirrors `active_revalidate`); polling continues.

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::store::{LicenseStore, account};
use keylight::{Keylight, KeylightConfig, LicenseState};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

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

/// Signed `v3` lease JSON (camelCase wire shape) with the given status/expiry
/// and entitlement set.
fn lease_json(signing: &SigningKey, status: &str, expires_at: i64, entitlements: &[&str]) -> String {
    let mut ents: Vec<&str> = entitlements.to_vec();
    ents.sort_unstable();
    let payload = format!("v3|{KID}|hash|i1|0|{expires_at}|{status}|{}", ents.join(","));
    let sig = signing.sign(payload.as_bytes());
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
    serde_json::json!({
        "kid": KID, "licenseKeyHash": "hash", "instanceId": "i1",
        "issuedAt": 0, "expiresAt": expires_at, "status": status,
        "signature": sig_b64, "entitlements": entitlements,
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

/// A store holding a signature-valid, currently-active lease with the given
/// entitlements, validated online just now.
fn store_with_active_lease(dir: &str, signing: &SigningKey, entitlements: &[&str]) -> Arc<EncryptedFileStore> {
    let store = empty_store(dir);
    store.set_string(account::LICENSE_KEY, "PRO-KEY").unwrap();
    store.set_string(account::INSTANCE_ID, "i1").unwrap();
    store
        .set_string(
            account::LEASE,
            &lease_json(signing, "active", now() + 100_000, entitlements),
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

/// Counting transport: replies with a fixed status + body on every call.
struct Counting {
    response: (u16, String),
    calls: AtomicUsize,
}
impl Counting {
    fn new(status: u16, body: &str) -> Arc<Self> {
        Arc::new(Self {
            response: (status, body.into()),
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
            status: self.response.0,
            body: self.response.1.clone(),
            retry_after: None,
        })
    }
    fn get(&self, _: &str, _: &[(String, String)]) -> TransportOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        TransportOutcome::Response(HttpResponse {
            status: self.response.0,
            body: self.response.1.clone(),
            retry_after: None,
        })
    }
}

/// Scripted transport: the first `flip_after` calls return one response, every
/// call after that returns a second one — used to land a rejection (or any
/// other change) partway through a poll loop rather than on the first attempt.
struct Scripted {
    flip_after: usize,
    before_flip: (u16, String),
    after_flip: (u16, String),
    calls: AtomicUsize,
}
impl Scripted {
    fn new(flip_after: usize, before_flip: (u16, &str), after_flip: (u16, &str)) -> Arc<Self> {
        Arc::new(Self {
            flip_after,
            before_flip: (before_flip.0, before_flip.1.into()),
            after_flip: (after_flip.0, after_flip.1.into()),
            calls: AtomicUsize::new(0),
        })
    }
    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl Transport for Scripted {
    fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> TransportOutcome {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let (status, body) = if n <= self.flip_after {
            &self.before_flip
        } else {
            &self.after_flip
        };
        TransportOutcome::Response(HttpResponse {
            status: *status,
            body: body.clone(),
            retry_after: None,
        })
    }
    fn get(&self, _: &str, _: &[(String, String)]) -> TransportOutcome {
        self.post_json("", &[], "")
    }
}

/// (1) The entitlement set changes on the very first validate → `true`, and
/// polling stops immediately (exactly one network call).
#[test]
fn entitlements_change_on_first_validate_returns_true() {
    let signing = signing_key();
    let store = store_with_active_lease("kl-rau-first-change", &signing, &["pro"]);
    let lease = lease_json(&signing, "active", now() + 100_000, &["pro", "enterprise"]);
    let body = format!(r#"{{"valid":true,"lease":{lease},"license_expires_at":{}}}"#, now() + 100_000);
    let transport = Counting::new(200, &body);
    let kl = Keylight::with_parts(config_trusting(&signing), store, transport.clone());

    let result = kl.refresh_after_upgrade(Duration::from_millis(250), Duration::from_millis(10));

    assert!(result, "new entitlements on the first poll must return true");
    assert_eq!(transport.count(), 1, "must stop polling as soon as a change lands");
}

/// (2) No stored license → `false`, and it must not touch the network at all.
#[test]
fn no_stored_license_returns_false_with_no_network_call() {
    let signing = signing_key();
    let store = empty_store("kl-rau-no-license");
    let transport = Counting::new(200, r#"{"valid":true}"#);
    let kl = Keylight::with_parts(config_trusting(&signing), store, transport.clone());

    let result = kl.refresh_after_upgrade(Duration::from_millis(250), Duration::from_millis(10));

    assert!(!result, "no stored license must return false");
    assert_eq!(transport.count(), 0, "no stored license must not hit the network");
}

/// (5) The webhook never lands: every validate echoes the same entitlements
/// and state, so this must run out the clock and return `false`, having
/// polled more than once (proving it actually polled, not just tried once).
#[test]
fn webhook_never_lands_times_out_after_polling_more_than_once() {
    let signing = signing_key();
    let store = store_with_active_lease("kl-rau-timeout", &signing, &["pro"]);
    let lease = lease_json(&signing, "active", now() + 100_000, &["pro"]);
    let body = format!(r#"{{"valid":true,"lease":{lease},"license_expires_at":{}}}"#, now() + 100_000);
    let transport = Counting::new(200, &body);
    let kl = Keylight::with_parts(config_trusting(&signing), store, transport.clone());

    let result = kl.refresh_after_upgrade(Duration::from_millis(250), Duration::from_millis(10));

    assert!(!result, "an unchanged echo must time out, not return true");
    assert!(
        transport.count() > 1,
        "must have polled more than once before giving up (got {})",
        transport.count()
    );
}

/// (4) A definitive rejection lands on the *second* poll (not the first, to
/// prove the loop actually polled): the license downgrades away from
/// `Licensed`, which is itself a state change → `true`.
#[test]
fn definitive_rejection_mid_poll_returns_true_and_downgrades_state() {
    let signing = signing_key();
    let store = store_with_active_lease("kl-rau-rejection", &signing, &["pro"]);
    let unchanged_lease = lease_json(&signing, "active", now() + 100_000, &["pro"]);
    let unchanged_body = format!(
        r#"{{"valid":true,"lease":{unchanged_lease},"license_expires_at":{}}}"#,
        now() + 100_000
    );
    let rejected_body = r#"{"valid":false,"error":"License downgraded"}"#;
    let transport = Scripted::new(1, (200, &unchanged_body), (422, rejected_body));
    let kl = Keylight::with_parts(config_trusting(&signing), store.clone(), transport.clone());

    assert_eq!(
        kl.state(),
        LicenseState::Licensed,
        "precondition: cached lease starts out valid"
    );

    let result = kl.refresh_after_upgrade(Duration::from_millis(250), Duration::from_millis(10));

    assert!(result, "a mid-poll rejection is a state change and must return true");
    assert_ne!(
        kl.state(),
        LicenseState::Licensed,
        "the rejection must have downgraded the session"
    );
    assert!(
        transport.count() >= 2,
        "the rejection must land after at least one unchanged poll (got {})",
        transport.count()
    );
}
