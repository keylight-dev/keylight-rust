//! Cross-SDK revocation & offline-bound parity (see design doc
//! `2026-07-08-cross-sdk-revocation-parity-design.md`):
//!
//! 1. `check_on_launch` must always validate against the server (no staleness
//!    gate), so a dashboard revoke lands on the next launch.
//! 2. A transient/network failure during that validate must never mutate state —
//!    the host keeps running on the last-known-good cached lease.
//! 3. `state()` must deny once `now - last_validated_online > max_offline_days`
//!    (default 15), even with a signature-valid, unexpired cached lease.
//! 4. `max_offline_days = None` disables the offline cap entirely.

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::store::{LicenseStore, account};
use keylight::{Keylight, KeylightConfig, LicenseState};
use std::sync::Arc;

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

/// Build the JSON object (camelCase, matching the wire `Lease` shape) for a
/// signed lease with the given status/expiry.
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

/// A config trusting `signing`'s public key, with `max_offline_days` overridden
/// after `build()` (the builder only exposes an enabling setter; the field
/// itself is public so a consumer can still opt out of the cap entirely).
fn config_trusting(signing: &SigningKey, max_offline_days: Option<u32>) -> KeylightConfig {
    let pub_b64 =
        base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes());
    let mut cfg = KeylightConfig::builder("t", "p", "sdk_live_test")
        .trusted_key(KID, pub_b64)
        .build();
    cfg.max_offline_days = max_offline_days;
    cfg
}

/// A store with a signature-valid, currently-active lease and the given
/// `last_validated_online` anchor already persisted.
fn store_with_active_lease(
    dir: &str,
    signing: &SigningKey,
    last_validated_online: i64,
) -> Arc<EncryptedFileStore> {
    let d = std::env::temp_dir().join(dir);
    let _ = std::fs::remove_dir_all(&d);
    let store =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
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
        .set_string(
            account::LAST_VALIDATED_ONLINE,
            &last_validated_online.to_string(),
        )
        .unwrap();
    store
}

/// Returns a fixed HTTP 200 body verbatim for `post_json`; `get` is unused here.
struct FixedResponse(String);
impl Transport for FixedResponse {
    fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> TransportOutcome {
        TransportOutcome::Response(HttpResponse {
            status: 200,
            body: self.0.clone(),
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

/// Always fails the transport with a non-retryable transport-level error
/// (simulates offline/DNS failure — no HTTP response at all).
struct AlwaysDown;
impl Transport for AlwaysDown {
    fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> TransportOutcome {
        TransportOutcome::Terminal("offline".into())
    }
    fn get(&self, _: &str, _: &[(String, String)]) -> TransportOutcome {
        TransportOutcome::Terminal("offline".into())
    }
}

/// Returns a fixed HTTP status + body verbatim for `post_json` (used to
/// simulate the real worker's 422 revoke/deactivation response).
struct FixedStatusResponse(u16, String);
impl Transport for FixedStatusResponse {
    fn post_json(&self, _: &str, _: &[(String, String)], _: &str) -> TransportOutcome {
        TransportOutcome::Response(HttpResponse {
            status: self.0,
            body: self.1.clone(),
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

/// (a) Revoke caught on launch: a cached, currently-active lease plus a server
/// that now returns a definitive rejection (with a signed `expired`-status
/// lease, mirroring how the API communicates revocation) must deny after
/// `check_on_launch`, not lag behind on the old cached lease.
#[test]
fn revoke_is_caught_on_launch() {
    let signing = signing_key();
    let store = store_with_active_lease("kl-launch-revoke", &signing, now());
    let cfg = config_trusting(&signing, Some(15));

    let revoked_lease = lease_json(&signing, "expired", now() - 1000);
    let body = format!(r#"{{"valid":false,"lease":{revoked_lease}}}"#);
    let kl = Keylight::with_parts(cfg, store, Arc::new(FixedResponse(body)));

    assert_eq!(
        kl.state(),
        LicenseState::Licensed,
        "precondition: cached lease starts out valid"
    );

    kl.check_on_launch()
        .expect("check_on_launch should not error on a definitive server rejection");

    assert_ne!(
        kl.state(),
        LicenseState::Licensed,
        "a dashboard revoke must be caught on the very next launch"
    );
}

/// (b) Transient failure keeps access: a `check_on_launch` whose validate call
/// hits a transport-level failure must not deny or mutate the cached lease —
/// the host keeps running on the last-known-good state (within the offline cap).
#[test]
fn transient_failure_keeps_access_within_cap() {
    let signing = signing_key();
    let store = store_with_active_lease("kl-launch-transient", &signing, now());
    let cfg = config_trusting(&signing, Some(15));
    let lease_before = store.get_string(account::LEASE);
    let kl = Keylight::with_parts(cfg, store.clone(), Arc::new(AlwaysDown));

    let result = kl.check_on_launch();
    assert!(
        result.is_err(),
        "a genuine transport failure should surface as an error to the caller"
    );

    assert_eq!(
        kl.state(),
        LicenseState::Licensed,
        "a transient failure must not deny a license that is still within the offline cap"
    );
    assert_eq!(
        store.get_string(account::LEASE),
        lease_before,
        "a transient failure must not mutate the cached lease"
    );
}

/// (c) Past the offline cap denies: `last_validated_online` older than the
/// default 15-day cap must deny via `state()`, even though the cached lease
/// itself is signature-valid and not yet expired.
#[test]
fn past_offline_cap_denies_even_with_valid_cached_lease() {
    let signing = signing_key();
    let sixteen_days_ago = now() - 16 * 86400;
    let store = store_with_active_lease("kl-launch-cap-exceeded", &signing, sixteen_days_ago);
    let cfg = config_trusting(&signing, Some(15));
    let kl = Keylight::with_parts(cfg, store, Arc::new(AlwaysDown));

    assert_ne!(
        kl.state(),
        LicenseState::Licensed,
        "a license unseen by the server for >15 days must not resolve as Licensed"
    );
}

/// (e) Real dashboard revoke / deactivated-instance shape: HTTP 422, body
/// `{"error": "Instance not found or deactivated"}` — no `lease` field and no
/// `valid` field at all (the worker omits it entirely on this path, unlike the
/// synthetic `"valid":false` bodies used elsewhere in this file). `valid` must
/// default to `false` on deserialization rather than fail, and since there is
/// no lease to fall back to, the stale cached "active" lease must be cleared
/// so `state()` denies instead of continuing to report Licensed off stale data.
#[test]
fn real_422_no_lease_revoke_clears_stale_lease_and_denies() {
    let signing = signing_key();
    let store = store_with_active_lease("kl-launch-422-no-lease", &signing, now());
    let cfg = config_trusting(&signing, Some(15));
    let kl = Keylight::with_parts(
        cfg,
        store.clone(),
        Arc::new(FixedStatusResponse(
            422,
            r#"{"error":"Instance not found or deactivated"}"#.into(),
        )),
    );

    assert_eq!(
        kl.state(),
        LicenseState::Licensed,
        "precondition: cached lease starts out valid"
    );

    kl.check_on_launch()
        .expect("a definitive 422 rejection is not a transport error");

    assert_ne!(
        kl.state(),
        LicenseState::Licensed,
        "a real revoke/deactivation (422, no lease, no `valid` field) must deny"
    );
    assert!(
        store.get_string(account::LEASE).is_none(),
        "the stale 'active' lease must be cleared, not left behind for state() to reuse"
    );
}

/// (f) Real 422 rejection that *does* carry a lease (e.g. `"expired"` status,
/// which the worker sends on some definitive-rejection paths): that lease must
/// still be stored so `state()` can resolve it (expired), matching existing
/// fallback/expired handling — this must not regress when the no-lease branch
/// above starts clearing the store.
#[test]
fn real_422_with_expired_lease_keeps_lease_and_resolves_expired() {
    let signing = signing_key();
    let store = store_with_active_lease("kl-launch-422-with-lease", &signing, now());
    let cfg = config_trusting(&signing, Some(15));
    let expired_lease = lease_json(&signing, "expired", now() - 1000);
    let body = format!(r#"{{"valid":false,"lease":{expired_lease}}}"#);
    let kl = Keylight::with_parts(cfg, store.clone(), Arc::new(FixedStatusResponse(422, body)));

    kl.check_on_launch()
        .expect("a definitive 422 rejection is not a transport error");

    assert_eq!(
        kl.state(),
        LicenseState::Expired,
        "a 422 with an expired lease must resolve to Expired, with the lease kept"
    );
    assert!(
        store.get_string(account::LEASE).is_some(),
        "the server-sent expired lease must be persisted, not cleared"
    );
}

/// (d) Cap disabled: `max_offline_days = None` must never deny for age, however
/// long it has been since the last successful online validation.
#[test]
fn disabled_cap_never_denies_for_age() {
    let signing = signing_key();
    let ancient = now() - 1000 * 86400;
    let store = store_with_active_lease("kl-launch-cap-disabled", &signing, ancient);
    let cfg = config_trusting(&signing, None);
    let kl = Keylight::with_parts(cfg, store, Arc::new(AlwaysDown));

    assert_eq!(
        kl.state(),
        LicenseState::Licensed,
        "max_offline_days = None must disable the offline cap entirely"
    );
}
