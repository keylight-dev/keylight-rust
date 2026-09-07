//! `require_signed_config` — the enforcement half of server-owned settings.
//!
//! Two rules from section 4 of `keylight-cpp`'s
//! `docs/superpowers/specs/2026-09-05-trial-parity-handoff.md` are what these
//! tests exist to hold:
//!
//! - **Rule 2:** a config that does not verify is *never cached*. It falls back
//!   to the seed, never to what the server claimed.
//! - **Rule 4:** the rule applies wherever the fields ride. If `/config`
//!   verifies but a `validate` body caches unsigned settings, the check is one
//!   route away from useless. Authentication is a property of the fields, not
//!   of the endpoint they arrived on.
//!
//! The flag defaults **off**: the worker signs a product's settings only once
//! that product has a trial length configured, so defaulting it on would reject
//! the legitimate unsigned responses every other product gets.

use ed25519_dalek::{Signer, SigningKey};
use keylight::http::{HttpResponse, Transport, TransportOutcome};
use keylight::store::device::FixedDeviceIdentity;
use keylight::store::encrypted_file::EncryptedFileStore;
use keylight::store::{LicenseStore, account};
use keylight::{Keylight, KeylightConfig};
use std::sync::{Arc, Mutex};

const TENANT: &str = "t";
const PRODUCT: &str = "p";
const KID: &str = "k1";

struct Scripted {
    body: String,
    paths: Mutex<Vec<String>>,
}

impl Scripted {
    fn new(body: String) -> Arc<Self> {
        Arc::new(Self {
            body,
            paths: Mutex::new(Vec::new()),
        })
    }
    fn respond(&self, url: &str) -> TransportOutcome {
        self.paths.lock().unwrap().push(url.to_string());
        TransportOutcome::Response(HttpResponse {
            status: 200,
            body: self.body.clone(),
            retry_after: None,
        })
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

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn signing_key() -> SigningKey {
    // Fixed seed: these tests assert on verification outcomes, not on key
    // generation, and a deterministic key keeps failures reproducible.
    let mut seed = [0u8; 32];
    for (i, b) in seed.iter_mut().enumerate() {
        *b = (i + 1) as u8;
    }
    SigningKey::from_bytes(&seed)
}

fn public_key_b64(key: &SigningKey) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(key.verifying_key().to_bytes())
}

/// Sign the way the worker does, so these tests exercise the real payload rather
/// than a shape invented here.
fn signed_body(trial_days: u32, free_tier: bool, tenant: &str, product: &str) -> String {
    use base64::Engine;
    let key = signing_key();
    let issued_at = now();
    let expires_at = issued_at + 86_400;
    let payload = format!(
        "cfg1|{KID}|{tenant}|{product}|{issued_at}|{expires_at}|{trial_days}|{}",
        if free_tier { "true" } else { "false" }
    );
    let sig =
        base64::engine::general_purpose::STANDARD.encode(key.sign(payload.as_bytes()).to_bytes());
    format!(
        r#"{{"valid":true,"trial_duration_days":{trial_days},"free_tier_enabled":{free_tier},"issued_at":{issued_at},"expires_at":{expires_at},"kid":"{KID}","signature":"{sig}"}}"#
    )
}

fn unsigned_body(trial_days: u32) -> String {
    format!(r#"{{"valid":true,"trial_duration_days":{trial_days},"free_tier_enabled":false}}"#)
}

struct Harness {
    client: Keylight,
    #[allow(dead_code)]
    store: Arc<dyn LicenseStore>,
}

fn harness(
    dir: &str,
    seed_days: u32,
    require_signed: bool,
    body: String,
    trusted_kid: &str,
) -> Harness {
    // Unique per process: fixed names collide when two `cargo test` runs overlap,
    // which showed up once as a spurious failure.
    let d = std::env::temp_dir().join(format!("{dir}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let store: Arc<dyn LicenseStore> =
        Arc::new(EncryptedFileStore::at_dir(d, &FixedDeviceIdentity("dev".into())).unwrap());
    let mut cfg = KeylightConfig::builder(TENANT, PRODUCT, "sdk_live_test")
        .trial_duration_days(seed_days)
        .require_signed_config(require_signed)
        .build();
    cfg.trusted_keys
        .insert(trusted_kid.to_string(), public_key_b64(&signing_key()));
    // validate() returns NoStoredLicense before touching the wire without these,
    // so a test that did not seed them would never exercise the body at all.
    store
        .set_string(account::LICENSE_KEY, "KL-TEST-AAAA-BBBB")
        .unwrap();
    store.set_string(account::INSTANCE_ID, "inst-001").unwrap();
    Harness {
        client: Keylight::with_parts(cfg, Arc::clone(&store), Scripted::new(body)),
        store,
    }
}

// ------------------------------------------------------------------ default off

#[test]
fn enforcement_is_off_by_default() {
    let cfg = KeylightConfig::builder(TENANT, PRODUCT, "sdk_live_test").build();
    assert!(!cfg.require_signed_config);
}

#[test]
fn unsigned_config_is_absorbed_when_enforcement_is_off() {
    let h = harness("kl-enf-off", 0, false, unsigned_body(14), KID);
    h.client.fetch_config();
    assert_eq!(h.client.effective_trial_duration_days(), 14);
}

// --------------------------------------------- rule 2: never cache what failed

#[test]
fn unsigned_config_is_rejected_when_enforcement_is_on() {
    let h = harness("kl-enf-unsigned", 7, true, unsigned_body(365), KID);
    h.client.fetch_config();
    assert_eq!(
        h.client.effective_trial_duration_days(),
        7,
        "rule 2: fall back to the seed, never to what the server claimed"
    );
}

#[test]
fn validly_signed_config_is_absorbed_when_enforcement_is_on() {
    let h = harness(
        "kl-enf-signed",
        7,
        true,
        signed_body(14, false, TENANT, PRODUCT),
        KID,
    );
    h.client.fetch_config();
    assert_eq!(h.client.effective_trial_duration_days(), 14);
}

#[test]
fn config_signed_by_a_rotated_away_kid_is_rejected() {
    let h = harness(
        "kl-enf-rotated",
        7,
        true,
        signed_body(14, false, TENANT, PRODUCT),
        "k2",
    );
    h.client.fetch_config();
    assert_eq!(h.client.effective_trial_duration_days(), 7);
}

#[test]
fn config_signed_for_another_product_is_rejected() {
    let h = harness(
        "kl-enf-neighbour",
        7,
        true,
        signed_body(14, false, TENANT, "neighbourapp"),
        KID,
    );
    h.client.fetch_config();
    assert_eq!(h.client.effective_trial_duration_days(), 7);
}

// ------------------------------------- rule 4: validate is not a way around it

/// The bypass this design exists to prevent: if `validate` wrote the cache
/// without checking, stripping the signature from that one route would be enough
/// to set any trial length you liked.
#[test]
fn unsigned_validate_body_cannot_write_the_cache_when_enforcement_is_on() {
    let h = harness("kl-enf-validate-unsigned", 7, true, unsigned_body(365), KID);
    let _ = h.client.validate();
    assert_eq!(
        h.client.effective_trial_duration_days(),
        7,
        "an unsigned validate body must not be a way around the gate"
    );
}

#[test]
fn signed_validate_body_is_accepted_when_enforcement_is_on() {
    let h = harness(
        "kl-enf-validate-signed",
        7,
        true,
        signed_body(21, false, TENANT, PRODUCT),
        KID,
    );
    let _ = h.client.validate();
    assert_eq!(
        h.client.effective_trial_duration_days(),
        21,
        "validate must carry its signature through to the gate"
    );
}
