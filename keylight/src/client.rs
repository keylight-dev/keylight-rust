//! The [`Keylight`] client: activation, validation, deactivation, offline state
//! resolution, trials, the keyless beacon, refresh timing, and lifecycle events.

use crate::clock::{clock_manipulated, clock_rolled_back};
use crate::http::retry::{MAX_ATTEMPTS, RetryDecision, backoff_ms, clamp_sleep_ms, decide};
use crate::http::{Transport, TransportOutcome, ureq_transport::UreqTransport};
use crate::state::{KeylessState, LicenseState, TrialStatus, resolve_state};
use crate::store::device::{DeviceIdentity, SystemDeviceIdentity};
use crate::store::{LicenseStore, account, encrypted_file::EncryptedFileStore};
use crate::{KeylightConfig, KeylightError, Lease, Result, telemetry, verify_lease};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ActivationResult {
    pub activated: bool,
    pub instance_id: Option<String>,
    pub lease: Option<Lease>,
    pub license_expires_at: Option<i64>,
    pub error: Option<String>,
}
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub valid: bool,
    pub lease: Option<Lease>,
    pub license_expires_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct ActivateResp {
    activated: bool,
    instance_id: Option<String>,
    license_expires_at: Option<i64>,
    lease: Option<Lease>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct ValidateResp {
    /// Defaults to `false` when absent: the real worker's revoked /
    /// instance-not-active response is `{"error": "..."}` with no `valid`
    /// field at all, and that must be treated as a definitive rejection,
    /// not fail to deserialize.
    #[serde(default)]
    valid: bool,
    license_expires_at: Option<i64>,
    lease: Option<Lease>,
    error: Option<String>,
}
#[derive(Deserialize)]
struct ErrorResp {
    error: Option<String>,
}

pub struct Keylight {
    config: KeylightConfig,
    store: Arc<dyn LicenseStore>,
    transport: Arc<dyn Transport>,
    device: Arc<dyn DeviceIdentity>,
    on_event: Option<Box<dyn Fn(crate::state::LicenseLifecycleEvent) + Send + Sync>>,
}

impl Keylight {
    /// Construct with the default encrypted-file store + ureq transport.
    pub fn new(config: KeylightConfig) -> Result<Self> {
        let ns = format!("{}-{}", config.tenant_id, config.product_id);
        Ok(Self {
            store: Arc::new(EncryptedFileStore::new(&ns)?),
            transport: Arc::new(UreqTransport::default()),
            device: Arc::new(SystemDeviceIdentity),
            config,
            on_event: None,
        })
    }
    /// Construct with custom store + transport (tests, alternate backends).
    pub fn with_parts(
        config: KeylightConfig,
        store: Arc<dyn LicenseStore>,
        transport: Arc<dyn Transport>,
    ) -> Self {
        Self {
            config,
            store,
            transport,
            device: Arc::new(SystemDeviceIdentity),
            on_event: None,
        }
    }
    /// Register a handler invoked when the resolved license state crosses a lifecycle transition.
    pub fn with_event_handler(
        mut self,
        handler: impl Fn(crate::state::LicenseLifecycleEvent) + Send + Sync + 'static,
    ) -> Self {
        self.on_event = Some(Box::new(handler));
        self
    }
    /// Override the device identity used for `machine_hash` on the keyless heartbeat
    /// (tests, alternate platforms). Defaults to [`SystemDeviceIdentity`].
    pub fn with_device(mut self, device: Arc<dyn DeviceIdentity>) -> Self {
        self.device = device;
        self
    }

    fn request_id() -> String {
        use rand::Rng;
        let n: u32 = rand::thread_rng().r#gen();
        format!("{n:08x}")
    }
    fn headers(&self) -> Vec<(String, String)> {
        let mut h = vec![
            ("Content-Type".into(), "application/json".into()),
            ("X-Keylight-Request-Id".into(), Self::request_id()),
        ];
        if !self.config.sdk_key.is_empty() {
            h.push(("X-Keylight-SDK-Key".into(), self.config.sdk_key.clone()));
        }
        h
    }
    fn body_with_telemetry(&self, mut map: serde_json::Map<String, serde_json::Value>) -> String {
        telemetry::apply(&mut map, self.config.app_version.as_deref());
        serde_json::Value::Object(map).to_string()
    }

    /// True hardware id with a persisted cache: a fresh OS read wins (and refreshes the
    /// cache); on a transient read failure the last successfully read id is reused so the
    /// derived `machine_hash` stays stable across beacons. NO random fallback — if no id
    /// has ever been read this returns `None` and callers omit the field.
    fn cached_hardware_id(&self) -> Option<String> {
        match self.device.hardware_id() {
            Some(hw) => {
                let _ = self.store.set_string(account::CACHED_HARDWARE_ID, &hw);
                Some(hw)
            }
            None => self.store.get_string(account::CACHED_HARDWARE_ID),
        }
    }
    /// Cross-SDK `machine_hash` (lowercase hex) from the cached hardware id, if any.
    fn machine_hash(&self) -> Option<String> {
        self.cached_hardware_id().map(|hw| {
            crate::machine::machine_hash(&self.config.tenant_id, &self.config.product_id, &hw)
        })
    }

    /// POST with retry/backoff. `decodable_4xx` lets a caller opt a 4xx body in (validate's 422).
    fn post(&self, path: &str, body: &str, decodable_4xx: &[u16]) -> Result<(u16, String)> {
        let url = self.api_url(path);
        let headers = self.headers();
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match self.transport.post_json(&url, &headers, body) {
                TransportOutcome::Response(r) => {
                    if r.status == 200 || decodable_4xx.contains(&r.status) {
                        return Ok((r.status, r.body));
                    }
                    match decide(r.status, attempt, r.retry_after) {
                        RetryDecision::RetryAfter(ms) => {
                            std::thread::sleep(std::time::Duration::from_millis(ms + jitter_ms()));
                            continue;
                        }
                        RetryDecision::Stop => {
                            if r.status == 429 {
                                return Err(KeylightError::RateLimited {
                                    retry_after: r.retry_after.unwrap_or(0),
                                });
                            }
                            if (500..=599).contains(&r.status) || r.status == 408 {
                                return Err(KeylightError::ServerError { status: r.status });
                            }
                            let msg = serde_json::from_str::<ErrorResp>(&r.body)
                                .ok()
                                .and_then(|e| e.error)
                                .unwrap_or_default();
                            return Err(KeylightError::ClientError {
                                status: r.status,
                                message: msg,
                            });
                        }
                    }
                }
                TransportOutcome::Transient(_) if attempt < MAX_ATTEMPTS => {
                    std::thread::sleep(std::time::Duration::from_millis(
                        clamp_sleep_ms(backoff_ms(attempt)) + jitter_ms(),
                    ));
                    continue;
                }
                TransportOutcome::Transient(e) | TransportOutcome::Terminal(e) => {
                    return Err(KeylightError::NetworkFailure(e));
                }
                TransportOutcome::Timeout => return Err(KeylightError::Timeout),
            }
        }
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn api_url(&self, path: &str) -> String {
        format!(
            "{}/{}/{}/{}",
            self.config.base_url, self.config.tenant_id, self.config.product_id, path
        )
    }

    /// Verify a lease against the configured trusted keys at the current time.
    fn verify(&self, lease: &Lease) -> crate::VerifyResult {
        verify_lease(
            lease,
            &self.config.trusted_keys,
            Self::now(),
            crate::SKEW_SECONDS,
        )
    }

    fn verify_or_reject(&self, lease: &Lease) -> Result<()> {
        if self.verify(lease).is_trusted() {
            Ok(())
        } else {
            Err(KeylightError::LeaseVerificationFailed)
        }
    }

    pub fn activate(&self, key: &str) -> Result<ActivationResult> {
        if !self.config.validate_key_format(key) {
            return Ok(ActivationResult {
                activated: false,
                instance_id: None,
                lease: None,
                license_expires_at: None,
                error: Some("Invalid license key format".into()),
            });
        }
        let machine = machine_name();
        let mut map = serde_json::Map::new();
        map.insert("license_key".into(), key.into());
        map.insert("instance_name".into(), machine.into());
        if let Some(ft) = self.store.get_string(account::FREE_TIER_INSTANCE_ID) {
            map.insert("free_tier_instance_id".into(), ft.into());
        }
        if let Some(hash) = self.machine_hash() {
            map.insert("machine_hash".into(), hash.into());
        }
        let body = self.body_with_telemetry(map);

        let (_, text) = match self.post("activate", &body, &[]) {
            Ok(v) => v,
            Err(KeylightError::ClientError { status, message }) => {
                return Ok(ActivationResult {
                    activated: false,
                    instance_id: None,
                    lease: None,
                    license_expires_at: None,
                    error: Some(if message.is_empty() {
                        format!("Activation failed (HTTP {status})")
                    } else {
                        message
                    }),
                });
            }
            Err(e) => return Err(e),
        };
        let resp: ActivateResp =
            serde_json::from_str(&text).map_err(|_| KeylightError::InvalidResponse)?;
        if !resp.activated {
            return Ok(ActivationResult {
                activated: false,
                instance_id: None,
                lease: None,
                license_expires_at: None,
                error: resp.error.or(Some("Activation failed".into())),
            });
        }
        if let Some(lease) = &resp.lease {
            self.verify_or_reject(lease)?;
        }

        self.store.set_string(account::LICENSE_KEY, key)?;
        if let Some(id) = &resp.instance_id {
            self.store.set_string(account::INSTANCE_ID, id)?;
        }
        if let Some(lease) = &resp.lease {
            self.store_lease(lease)?;
        }
        self.save_expiry(resp.license_expires_at)?;
        self.touch_last_seen()?;
        self.touch_validated_online()?;
        Ok(ActivationResult {
            activated: true,
            instance_id: resp.instance_id,
            lease: resp.lease,
            license_expires_at: resp.license_expires_at,
            error: None,
        })
    }

    pub fn validate(&self) -> Result<ValidationResult> {
        let key = self
            .store
            .get_string(account::LICENSE_KEY)
            .ok_or(KeylightError::NoStoredLicense)?;
        let instance = self
            .store
            .get_string(account::INSTANCE_ID)
            .ok_or(KeylightError::NoStoredLicense)?;
        let prev_state = self.state();
        let prev_expiry = self.store.get_i64(account::LICENSE_EXPIRES_AT);
        let mut map = serde_json::Map::new();
        map.insert("license_key".into(), key.into());
        map.insert("instance_id".into(), instance.into());
        if let Some(hash) = self.machine_hash() {
            map.insert("machine_hash".into(), hash.into());
        }
        let body = self.body_with_telemetry(map);

        let (_status, text) = match self.post("validate", &body, &[422]) {
            Ok(v) => v,
            Err(KeylightError::ClientError { status, message }) => {
                return Ok(ValidationResult {
                    valid: false,
                    lease: None,
                    license_expires_at: None,
                    error: Some(if message.is_empty() {
                        format!("Validation failed (HTTP {status})")
                    } else {
                        message
                    }),
                });
            }
            Err(e) => return Err(e),
        };
        let resp: ValidateResp =
            serde_json::from_str(&text).map_err(|_| KeylightError::InvalidResponse)?;
        if let Some(lease) = &resp.lease {
            self.verify_or_reject(lease)?;
        }
        if !resp.valid {
            // Definitive rejection: persist whatever lease the server sent (e.g.
            // "expired"/"fallback" so state() can resolve .limited/.expired), or
            // clear the cached one when it sent none at all. The real worker's
            // revoked/instance-not-active responses are `{"error": "..."}` with
            // no `lease` field, so leaving the old (still "active") lease in
            // place would let state() keep reporting Licensed off stale data.
            match &resp.lease {
                Some(lease) => self.store_lease(lease)?,
                None => self.store.delete(account::LEASE)?,
            }
            self.save_expiry(resp.license_expires_at)?;
            self.emit_lifecycle(&prev_state, prev_expiry);
            return Ok(ValidationResult {
                valid: false,
                lease: resp.lease,
                license_expires_at: resp.license_expires_at,
                error: resp.error,
            });
        }
        if let Some(lease) = &resp.lease {
            self.store_lease(lease)?;
        }
        self.save_expiry(resp.license_expires_at)?;
        self.touch_last_seen()?;
        self.touch_validated_online()?;
        self.emit_lifecycle(&prev_state, prev_expiry);
        Ok(ValidationResult {
            valid: true,
            lease: resp.lease,
            license_expires_at: resp.license_expires_at,
            error: None,
        })
    }

    pub fn deactivate(&self) -> Result<()> {
        let key = self.store.get_string(account::LICENSE_KEY);
        let instance = self.store.get_string(account::INSTANCE_ID);
        let mut net_err = None;
        if let (Some(k), Some(i)) = (key, instance) {
            let mut map = serde_json::Map::new();
            map.insert("license_key".into(), k.into());
            map.insert("instance_id".into(), i.into());
            let body = self.body_with_telemetry(map);
            if let Err(e) = self.post("deactivate", &body, &[]) {
                net_err = Some(e);
            }
        }
        for a in [
            account::LICENSE_KEY,
            account::INSTANCE_ID,
            account::LEASE,
            account::LICENSE_EXPIRES_AT,
            account::LAST_VALIDATED_ONLINE,
            account::LAST_SEEN,
        ] {
            self.store.delete(a)?;
        }
        net_err.map_or(Ok(()), Err)
    }

    pub fn cached_lease(&self) -> Option<Lease> {
        if let Some(max_days) = self.config.max_offline_days {
            let last = self.store.get_i64(account::LAST_VALIDATED_ONLINE)?;
            if Self::now() - last > (max_days as i64) * 86400 {
                return None;
            }
        }
        let lease: Lease = serde_json::from_str(&self.store.get_string(account::LEASE)?).ok()?;
        let r = self.verify(&lease);
        if r.is_trusted() && !r.expired && lease.status != "expired" {
            Some(lease)
        } else {
            None
        }
    }

    pub fn has_entitlement(&self, feature: &str) -> bool {
        self.cached_lease()
            .map(|l| l.entitlements.iter().any(|e| e == feature))
            .unwrap_or(false)
    }
    pub fn has_stored_license(&self) -> bool {
        self.store.get_string(account::LICENSE_KEY).is_some()
    }
    pub fn cached_license_key(&self) -> Option<String> {
        self.store.get_string(account::LICENSE_KEY)
    }
    /// The cached license expiry (epoch seconds), if one was stored on the last
    /// activate/validate. Parity with Swift `getCachedLicenseExpiresAt`.
    pub fn cached_license_expires_at(&self) -> Option<i64> {
        self.store.get_i64(account::LICENSE_EXPIRES_AT)
    }

    /// Persist a verified lease. Serializing a `Lease` (only owned strings, integers,
    /// and a string vec) cannot fail, so a serialization error here would be a logic
    /// bug rather than a recoverable condition.
    fn store_lease(&self, lease: &Lease) -> Result<()> {
        let json = serde_json::to_string(lease).expect("Lease serializes to JSON infallibly");
        self.store.set_string(account::LEASE, &json)
    }
    fn save_expiry(&self, e: Option<i64>) -> Result<()> {
        match e {
            Some(v) => self
                .store
                .set_string(account::LICENSE_EXPIRES_AT, &v.to_string()),
            None => self.store.delete(account::LICENSE_EXPIRES_AT),
        }
    }
    fn touch_last_seen(&self) -> Result<()> {
        self.store
            .set_string(account::LAST_SEEN, &Self::now().to_string())
    }
    fn touch_validated_online(&self) -> Result<()> {
        self.store
            .set_string(account::LAST_VALIDATED_ONLINE, &Self::now().to_string())
    }
}

impl Keylight {
    pub fn start_trial(&self) -> Result<()> {
        if self.store.get_string(account::TRIAL_START).is_none() {
            self.store
                .set_string(account::TRIAL_START, &Self::now().to_string())?;
        }
        if self
            .store
            .get_string(account::FREE_TIER_INSTANCE_ID)
            .is_none()
        {
            self.store.set_string(
                account::FREE_TIER_INSTANCE_ID,
                &crate::store::device::uuid_v4_pub(),
            )?;
        }
        Ok(())
    }
    pub fn check_trial(&self) -> TrialStatus {
        let start = match self.store.get_i64(account::TRIAL_START) {
            Some(v) => v,
            None => return TrialStatus::NotStarted,
        };
        let days_elapsed = (Self::now() - start) / 86400;
        let days_left = self.config.trial_duration_days as i64 - days_elapsed;
        if days_left > 0 {
            TrialStatus::Active { days_left }
        } else {
            TrialStatus::Expired
        }
    }
    pub fn is_clock_manipulated(&self) -> bool {
        let manipulated = self
            .store
            .get_i64(account::LAST_SEEN)
            .is_some_and(|last| clock_manipulated(last, Self::now()));
        if !manipulated {
            let _ = self.touch_last_seen();
        }
        manipulated
    }
    pub fn free_tier_instance_id(&self) -> Result<String> {
        if let Some(id) = self.store.get_string(account::FREE_TIER_INSTANCE_ID) {
            return Ok(id);
        }
        let id = crate::store::device::uuid_v4_pub();
        self.store.set_string(account::FREE_TIER_INSTANCE_ID, &id)?;
        Ok(id)
    }
    /// Anonymous keyless beacon, debounced 24h or on state change. Errors swallowed.
    pub fn report_keyless_state(&self, state: KeylessState) {
        let last_state = self.store.get_string(account::KEYLESS_LAST_STATE);
        let last_ping = self.store.get_i64(account::LAST_KEYLESS_PING_AT);
        let changed = last_state.as_deref() != Some(state.wire());
        let within = last_ping.map(|t| Self::now() - t < 86400).unwrap_or(false);
        if !changed && within {
            return;
        }
        let instance = match self.free_tier_instance_id() {
            Ok(i) => i,
            Err(_) => return,
        };
        let mut map = serde_json::Map::new();
        map.insert("instance_id".into(), instance.into());
        map.insert("state".into(), state.wire().into());
        if let Some(hash) = self.machine_hash() {
            map.insert("machine_hash".into(), hash.into());
        }
        let body = self.body_with_telemetry(map);
        // Route through the shared retry/backoff loop; with no decodable 4xx an
        // `Ok` here is exactly an HTTP 200, so the debounce state is persisted
        // only on success. Errors are swallowed (anonymous best-effort beacon).
        if self.post("keyless", &body, &[]).is_ok() {
            let _ = self
                .store
                .set_string(account::KEYLESS_LAST_STATE, state.wire());
            let _ = self
                .store
                .set_string(account::LAST_KEYLESS_PING_AT, &Self::now().to_string());
        }
    }
    /// Resolve the current high-level state from cached data (no network).
    pub fn state(&self) -> LicenseState {
        // Backward clock-rollback guard: if the system clock has jumped back more
        // than the tolerance since our last recorded contact, refuse to resolve a
        // usable state — this is the offline vector for reviving an expired lease.
        // Read-only (does not touch `last_seen`); the forward-jump component lives
        // in `is_clock_manipulated()`. Self-heals on the next successful
        // `validate()`, which re-anchors `last_seen`.
        if self
            .store
            .get_i64(account::LAST_SEEN)
            .is_some_and(|last| clock_rolled_back(last, Self::now()))
        {
            return LicenseState::Invalid;
        }
        // Offline bound: a validated license must not run forever without a
        // successful server re-check. When `max_offline_days` is configured the
        // cached lease is only usable if we have a `last_validated_online` anchor
        // within the cap. Both a *stale* anchor (older than the cap) and a
        // *missing* anchor are fail-closed — the latter matters because an
        // attacker who deletes the anchor to reset the offline clock must not
        // thereby revive the lease. This mirrors `cached_lease()` (whose `?` on
        // `get_i64` already short-circuits a missing anchor) and Swift's
        // `isWithinOfflineGrace`. When `max_offline_days` is `None` the cap is
        // disabled entirely (unlimited offline). Dropping the lease here lets a
        // stored license fall through to `Expired` via the `had_stored_license`
        // path in `resolve_state`, while trials / free-tier (no lease, no license)
        // are unaffected.
        let offline_bound_ok = match self.config.max_offline_days {
            Some(max_days) => self
                .store
                .get_i64(account::LAST_VALIDATED_ONLINE)
                .is_some_and(|last| Self::now() - last <= (max_days as i64) * 86400),
            None => true,
        };
        let lease = self
            .store
            .get_string(account::LEASE)
            .and_then(|s| serde_json::from_str::<Lease>(&s).ok());
        let (status, current) = match &lease {
            Some(l) if offline_bound_ok => {
                let r = self.verify(l);
                (r.is_trusted().then(|| l.status.clone()), !r.expired)
            }
            _ => (None, false),
        };
        resolve_state(
            status.as_deref(),
            current,
            self.has_stored_license(),
            &self.check_trial(),
            self.config.free_tier_enabled,
        )
    }
}

impl Keylight {
    /// Validate now only if enough time has passed (debounce 5min, stale 6h, or near expiry).
    pub fn refresh_if_needed(&self) -> Result<Option<ValidationResult>> {
        if !self.has_stored_license() {
            return Ok(None);
        }
        if let Some(last) = self.store.get_i64(account::LAST_VALIDATED_ONLINE) {
            let now = Self::now();
            if now - last < REFRESH_DEBOUNCE {
                return Ok(None);
            }
            let near_expiry = self
                .store
                .get_i64(account::LICENSE_EXPIRES_AT)
                .is_some_and(|exp| exp - now < 86400);
            if now - last < REFRESH_STALE && !near_expiry {
                return Ok(None);
            }
        }
        Ok(Some(self.validate()?))
    }
    /// Called on app launch: if a license is stored, **always** validate against
    /// the server (no staleness gate — unlike [`Self::refresh_if_needed`]), so a
    /// dashboard revoke or genuine expiry takes effect on the very next launch
    /// rather than lagging behind the in-session refresh cadence. `validate()`
    /// does not mutate state on a transient/network error, so a launch with no
    /// connectivity keeps running on the existing cached lease (last-known-good),
    /// subject to the offline bound enforced by [`Self::state`].
    pub fn check_on_launch(&self) -> Result<()> {
        if self.has_stored_license() {
            let _ = self.validate()?;
        }
        Ok(())
    }
    /// Hosted upgrade URL pre-filled with the cached key (parity with Swift upgradeURL).
    pub fn upgrade_url(&self) -> Option<String> {
        let key = self.cached_license_key()?;
        Some(format!(
            "https://portal.keylight.dev/p/{}/upgrade/{}?key={}",
            self.config.tenant_id,
            self.config.product_id,
            urlencode(&key)
        ))
    }

    /// Compute the post-validation state and fire a lifecycle event if the resolved
    /// state crossed a transition. The previous state is re-derived from the persisted
    /// lease on each call (so transitions don't re-fire across restarts). Errors swallowed.
    fn emit_lifecycle(&self, prev_state: &LicenseState, prev_expiry: Option<i64>) {
        let next_state = self.state();
        // Option<i64> ordering: None < Some(_), so this is true exactly when a new
        // expiry exists and is later than the previous one (or there was none).
        let expiry_moved_later = self.store.get_i64(account::LICENSE_EXPIRES_AT) > prev_expiry;
        if let Some(ev) = crate::state::lifecycle_event(prev_state, &next_state, expiry_moved_later)
        {
            if let Some(h) = &self.on_event {
                h(ev);
            }
        }
    }
}

const REFRESH_DEBOUNCE: i64 = 300; // 5 min
const REFRESH_STALE: i64 = 21600; // 6 h

fn urlencode(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// Best-effort human-readable machine name for the activation's `instance_name`
/// (display only — the seat identity is the server-issued `instance_id`). Falls back
/// through common env vars and the `hostname` command before a generic default.
fn machine_name() -> String {
    for var in ["HOSTNAME", "COMPUTERNAME", "HOST"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return v;
            }
        }
    }
    if let Ok(out) = std::process::Command::new("hostname").output() {
        let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !v.is_empty() {
            return v;
        }
    }
    "device".to_string()
}

/// Small random backoff jitter (0..250ms) to avoid synchronized retries
/// (the retry policy in `http::retry` stays pure; jitter is applied here).
fn jitter_ms() -> u64 {
    use rand::Rng;
    rand::thread_rng().gen_range(0..250)
}
