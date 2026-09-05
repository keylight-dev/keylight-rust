//! The [`Keylight`] client: activation, validation, deactivation, offline state
//! resolution, trials, the keyless beacon, refresh timing, and lifecycle events.

use crate::clock::{clock_manipulated, clock_rolled_back};
use crate::http::retry::{MAX_ATTEMPTS, RetryDecision, backoff_ms, clamp_sleep_ms, decide};
use crate::http::{Transport, TransportOutcome, ureq_transport::UreqTransport};
use crate::product_config::{CachedProductConfig, ProductConfigFields};
use crate::state::{KeylessState, LicenseState, TrialStatus, resolve_state};
use crate::store::device::{DeviceIdentity, SystemDeviceIdentity};
use crate::store::{LicenseStore, account, encrypted_file::EncryptedFileStore};
use crate::{KeylightConfig, KeylightError, Lease, Result, telemetry, verify_lease};
use serde::Deserialize;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

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
    /// Server-owned product settings, riding on a call the SDK already makes.
    /// Optional because an older worker sends neither.
    #[serde(default)]
    trial_duration_days: Option<u32>,
    #[serde(default)]
    free_tier_enabled: Option<bool>,
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
    /// Debounce anchor for [`Keylight::active_revalidate`] — monotonic, so a
    /// wall-clock change can neither stretch nor shrink the window. See that
    /// method for why the window is deliberately per-process and unpersisted.
    last_active_revalidate_at: Mutex<Option<Instant>>,
}

impl Keylight {
    /// Construct with the default encrypted-file store + ureq transport.
    pub fn new(config: KeylightConfig) -> Result<Self> {
        let ns = format!("{}-{}", config.tenant_id, config.product_id);
        let store = Arc::new(EncryptedFileStore::new(&ns)?);
        Ok(Self::with_parts(
            config,
            store,
            Arc::new(UreqTransport::default()),
        ))
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
            last_active_revalidate_at: Mutex::new(None),
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
    /// Telemetry plus the device dimensions (`os_version`, `arch`,
    /// `cpu_cores`, `memory`) — for the
    /// routes that describe a device (activate / validate / keyless).
    /// Deactivate stays on `body_with_telemetry`: it only names an instance.
    /// Reports the trial length this build was **compiled with** — the seed, not
    /// the effective value. Echoing the server's own number back diagnoses
    /// nothing; the seed catches the ordinary mistake of a 30-day build running
    /// against a 14-day dashboard setting, in a minute rather than a week of
    /// support tickets.
    ///
    /// Diagnostic only. The server must never gate on it: a patched client sends
    /// whatever its author wants, so a match proves nothing about the client.
    fn insert_seed_trial_telemetry(&self, map: &mut serde_json::Map<String, serde_json::Value>) {
        map.insert(
            "sdk_trial_duration_days".into(),
            self.config.trial_duration_days.into(),
        );
    }
    fn body_with_device_telemetry(
        &self,
        mut map: serde_json::Map<String, serde_json::Value>,
    ) -> String {
        telemetry::apply_device(&mut map);
        self.body_with_telemetry(map)
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
        self.insert_seed_trial_telemetry(&mut map);
        let body = self.body_with_device_telemetry(map);

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
        self.insert_seed_trial_telemetry(&mut map);
        let body = self.body_with_device_telemetry(map);

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
        // Absorb before the outcome branches below: the settings are valid
        // regardless of whether the licence itself validated, and an early
        // return would drop them for exactly the installs that keep failing.
        self.absorb_config_fields(&ProductConfigFields {
            trial_duration_days: resp.trial_duration_days,
            free_tier_enabled: resp.free_tier_enabled,
            ..Default::default()
        });
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
    /// Trial length actually in force: server value → local seed → 0.
    ///
    /// [`KeylightConfig::trial_duration_days`] is demoted to a *seed*, used only
    /// before this install has ever heard from the server. It is deliberately
    /// not removed: a brand-new install genuinely has nothing else, and dropping
    /// it would make first-launch behaviour depend on the network.
    pub fn effective_trial_duration_days(&self) -> u32 {
        self.cached_product_config()
            .trial_duration_days
            .unwrap_or(self.config.trial_duration_days)
    }

    /// Free-tier flag actually in force: server value → local seed → false.
    pub fn effective_free_tier_enabled(&self) -> bool {
        self.cached_product_config()
            .free_tier_enabled
            .unwrap_or(self.config.free_tier_enabled)
    }

    /// Explicitly refresh the product config from `GET /{tenant}/{product}/config`.
    ///
    /// **Not for the launch path.** The same two settings ride on `validate`
    /// (every licensed install) and on the keyless beacon (every unlicensed
    /// one), which is what keeps launch-time network I/O at zero. This exists
    /// for hosts that want an explicit refresh — a settings pane, a manual
    /// "check now" — and for tests.
    ///
    /// Failures are swallowed: a refresh that cannot reach the network leaves
    /// the last known settings in place rather than falling back to the seed.
    pub fn fetch_config(&self) {
        let url = self.api_url("config");
        let headers = self.headers();
        let text = match self.transport.get(&url, &headers) {
            TransportOutcome::Response(r) if r.status == 200 => r.body,
            _ => return,
        };
        if let Ok(fields) = serde_json::from_str::<ProductConfigFields>(&text) {
            self.absorb_config_fields(&fields);
        }
    }

    /// Merge server-sent settings into the cache, **field by field**.
    ///
    /// A response carrying neither field leaves the cache untouched — an older
    /// worker that knows nothing about these settings must not wipe what this
    /// install already learned. Each field is written only when the server
    /// actually sent it, rather than overwriting the pair.
    pub(crate) fn absorb_config_fields(&self, fields: &ProductConfigFields) {
        if fields.is_empty() {
            return;
        }
        let mut cached = self.cached_product_config();
        if let Some(days) = fields.trial_duration_days {
            cached.trial_duration_days = Some(days);
        }
        if let Some(free_tier) = fields.free_tier_enabled {
            cached.free_tier_enabled = Some(free_tier);
        }
        if let Ok(json) = serde_json::to_string(&cached) {
            let _ = self.store.set_string(account::PRODUCT_CONFIG, &json);
        }
    }

    pub(crate) fn cached_product_config(&self) -> CachedProductConfig {
        self.store
            .get_string(account::PRODUCT_CONFIG)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
}

impl Keylight {
    /// Stamps the trial start time, **unconditionally** — including when the
    /// effective duration is currently 0.
    ///
    /// This looks wrong and is not. Once the duration is server-owned, `0` is
    /// indistinguishable from "the config has not arrived yet", so returning
    /// early at a zero duration leaves no start timestamp for a later-arriving
    /// duration to measure, and the user never gets the trial their tenant
    /// enabled. The stamp grants nothing on its own: [`Keylight::check_trial`]
    /// still reports no trial while the effective duration is 0. It only fixes
    /// *when* the window starts if a duration arrives later.
    ///
    /// An existing stamp is never overwritten, so enabling a trial months after
    /// an install does not hand it a fresh window — otherwise it would be
    /// farmable by reinstalling.
    ///
    /// The anonymous instance id is minted at the same point for the same
    /// reason: nothing calls `start_trial()` a second time once the duration
    /// lands, so minting it only at a non-zero duration would lose attribution
    /// for any install that started offline.
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
        let days_left = self.effective_trial_duration_days() as i64 - days_elapsed;
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
        let body = self.body_with_device_telemetry(map);
        // Route through the shared retry/backoff loop; with no decodable 4xx an
        // `Ok` here is exactly an HTTP 200, so the debounce state is persisted
        // only on success. Errors are swallowed (anonymous best-effort beacon).
        if let Ok((_status, text)) = self.post("keyless", &body, &[]) {
            // The beacon response carries the product config for unlicensed
            // installs. A body that will not parse is ignored: the beacon is
            // best-effort and must not disturb the debounce bookkeeping below.
            if let Ok(fields) = serde_json::from_str::<ProductConfigFields>(&text) {
                self.absorb_config_fields(&fields);
            }
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
            self.effective_free_tier_enabled(),
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
    /// Force a re-validation on **active use** — app foreground, window focus,
    /// popover open — debounced to 60s in memory (parity with Swift
    /// `activeRevalidate()`).
    ///
    /// Unlike [`Self::refresh_if_needed`] this bypasses the debounce/stale/
    /// near-expiry gates entirely, so a dashboard revoke lands within minutes of
    /// the user next touching the app instead of waiting for the lease to go
    /// stale (up to 6h) or for a relaunch (up to the full lease lifetime).
    ///
    /// Behavior:
    /// - **No stored license key** — no-op, no network call, returns `None`.
    /// - **Inside the 60s window** — suppressed, returns `None`. The window is
    ///   in-memory only: it does not survive a process restart, and it is
    ///   consumed by the *attempt*, so a failed call also holds it (matching
    ///   Swift, and keeping a hammered foreground hook off the network).
    /// - **Definitive rejection** (`valid:false`, e.g. the revoke path's HTTP
    ///   422) — downgrades immediately through the same [`Self::validate`]
    ///   rejection branch, and returns `Some(result)` with `valid == false`.
    /// - **Transient failure** (offline, timeout, 5xx, rate limit) — returns
    ///   `None` with state untouched. This is the safety property: a network
    ///   blip must never downgrade a live session. `validate()` mutates nothing
    ///   on the error path, so the cached lease survives intact and access stays
    ///   governed by the offline bound in [`Self::state`].
    ///
    /// The error is intentionally swallowed rather than returned: this is a
    /// fire-and-forget UI hook whose whole contract is "never break the running
    /// session", so there is no failure a caller could act on differently. Use
    /// [`Self::validate`] directly when you need the error.
    pub fn active_revalidate(&self) -> Option<ValidationResult> {
        if !self.has_stored_license() {
            return None;
        }
        {
            // A panic while holding this lock is not reachable (the guarded
            // section is two moves), so recover from poisoning rather than
            // letting an unrelated panic elsewhere disable revalidation.
            let mut last = self
                .last_active_revalidate_at
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if last.is_some_and(|t| t.elapsed() < ACTIVE_REVALIDATE_DEBOUNCE) {
                return None;
            }
            *last = Some(Instant::now());
        }
        self.validate().ok()
    }

    /// Re-report the keyless beacon on a cadence for as long as the returned
    /// handle is alive.
    ///
    /// `report_keyless_state` has no cadence of its own, so a resident host — a
    /// daemon, a service, a desktop app built without the Tauri plugin —
    /// beacons once at startup and then looks dead to the dashboard for as long
    /// as it runs: `last_seen` never moves past `first_seen`. This is that
    /// cadence, for hosts the plugin does not cover.
    ///
    /// Each tick reports only when [`Keylight::state`] resolves to a keyless
    /// state; a licensed device sends nothing and reports liveness through
    /// `/validate`. The thread keeps ticking across that boundary, so a license
    /// that lapses resumes beaconing on its own. The 24h debounce inside
    /// `report_keyless_state` is untouched — this only guarantees the beacon
    /// gets the chance, so a tighter interval costs nothing on the wire.
    ///
    /// Six hours is the cross-SDK default; it matches the Worker's server-side
    /// gate on keyless writes, so a tick never arrives before the server is
    /// willing to record it. A zero interval is refused rather than spun on.
    ///
    /// Takes `Arc<Self>` because a thread cannot outlive a `&self` borrow. The
    /// thread holds only a [`Weak`], so it can never keep the client alive:
    /// drop the last `Arc` and the next tick exits. Dropping the handle stops
    /// and joins the thread, so it cannot outlive the caller's scope either.
    ///
    /// ```no_run
    /// # use std::{sync::Arc, time::Duration};
    /// # use keylight::{Keylight, KeylightConfig};
    /// # let cfg = KeylightConfig::builder("t", "p", "sdk_live_x").build();
    /// let kl = Arc::new(Keylight::new(cfg)?);
    /// let _heartbeat = kl.start_keyless_heartbeat(Duration::from_secs(6 * 60 * 60));
    /// // ... app runs; the beacon keeps reporting until `_heartbeat` drops.
    /// # Ok::<(), keylight::KeylightError>(())
    /// ```
    pub fn start_keyless_heartbeat(self: &Arc<Self>, interval: Duration) -> KeylessHeartbeat {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        if interval.is_zero() {
            // Nothing to schedule; the handle is inert but still valid to hold.
            return KeylessHeartbeat { stop, thread: None };
        }
        let weak = Arc::downgrade(self);
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let (lock, cv) = &*thread_stop;
            let mut stopped = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                // Interruptible wait: a dropped handle wakes this immediately
                // rather than leaving the caller blocked for a full interval.
                let (guard, _) = cv
                    .wait_timeout(stopped, interval)
                    .unwrap_or_else(|e| e.into_inner());
                stopped = guard;
                if *stopped {
                    return;
                }
                // Upgrade per tick: the client may have been dropped while we
                // waited, and holding it across the wait would defeat the Weak.
                let Some(kl) = weak.upgrade() else { return };
                if let Some(ks) = crate::state::keyless_state_for(&kl.state()) {
                    // Release the lock across the blocking network call so a
                    // drop during a send is not stuck behind it.
                    drop(stopped);
                    kl.report_keyless_state(ks);
                    stopped = lock.lock().unwrap_or_else(|e| e.into_inner());
                    if *stopped {
                        return;
                    }
                }
            }
        });
        KeylessHeartbeat {
            stop,
            thread: Some(thread),
        }
    }

    /// Poll-revalidate briefly after a customer completes an upgrade, so the new
    /// entitlements (or a mid-flight rejection) show up in the running app without
    /// waiting for the normal refresh cadence (parity with Swift's
    /// `LicenseManager.refreshAfterUpgrade(timeout:pollInterval:)`). This exists to
    /// cover payment-webhook lag: checkout can complete in the browser slightly
    /// before the provider's webhook reaches Keylight and the seat's lease actually
    /// changes server-side.
    ///
    /// Re-validates against the server every `poll_interval` (clamped to a 100ms
    /// floor) until `timeout` elapses. Returns `true` as soon as a call to
    /// [`Self::validate`] succeeds and either the entitlement *set* (order-independent)
    /// or the resolved [`Self::state`] differs from what it was when this method was
    /// called — including a definitive rejection landing mid-poll, since that always
    /// changes `state()`. Returns `false` on timeout or immediately, with no network
    /// call at all, when no license is stored. A transient/network error from
    /// `validate()` is not treated as a change (mirrors [`Self::active_revalidate`]);
    /// polling simply continues.
    ///
    /// **Blocking.** This sleeps between attempts and can take up to `timeout` to
    /// return — call it from a background thread (`std::thread::spawn`), never from
    /// a UI/main thread.
    ///
    /// A seat-only upgrade whose entitlement set and state end up identical to
    /// before (e.g. a device-cap bump with no feature/tier change) is invisible to
    /// this method: it will run to `timeout` and return `false`. The caller's normal
    /// refresh cadence ([`Self::refresh_if_needed`] / [`Self::check_on_launch`])
    /// still picks that change up on its own schedule.
    pub fn refresh_after_upgrade(&self, timeout: Duration, poll_interval: Duration) -> bool {
        if !self.has_stored_license() {
            return false;
        }
        let before_entitlements = Self::sorted_entitlements(self.cached_lease());
        let before_state = self.state();
        let poll = poll_interval.max(Duration::from_millis(100));
        let deadline = Instant::now() + timeout;
        loop {
            if self.validate().is_ok() {
                let now_entitlements = Self::sorted_entitlements(self.cached_lease());
                if now_entitlements != before_entitlements || self.state() != before_state {
                    return true;
                }
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(poll);
        }
    }
    /// Sorted entitlement list from an optional lease (empty when there is none),
    /// so callers can compare entitlement *sets* order-independently.
    fn sorted_entitlements(lease: Option<Lease>) -> Vec<String> {
        let mut ents = lease.map(|l| l.entitlements).unwrap_or_default();
        ents.sort_unstable();
        ents
    }

    /// Hosted upgrade page for the cached license, or `None` when no key is stored.
    ///
    /// Targets the portal's **authenticated** license route. The standalone
    /// public upgrade form is retired — `/p/{tenant}/...` now serves claim only —
    /// so a customer following this link signs in with a magic link and upgrades
    /// in-portal.
    ///
    /// The path segment is the NORMALIZED key (whitespace and dashes stripped,
    /// uppercased), because that is what the portal matches against
    /// `licenses.normalizedKey`. Normalizing also keeps the segment inside the
    /// route's `[A-Za-z0-9_-]` charset, so there is nothing left to
    /// percent-encode. The product is not in the URL: the license itself carries
    /// its product server-side.
    pub fn upgrade_url(&self) -> Option<String> {
        let key = self.cached_license_key()?;
        Some(format!(
            "https://portal.keylight.dev/t/{}/license/{}/upgrade",
            self.config.tenant_id,
            normalize_key(&key)
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
/// In-memory floor between two `active_revalidate()` network calls (Swift parity).
const ACTIVE_REVALIDATE_DEBOUNCE: Duration = Duration::from_secs(60);

/// RAII handle for [`Keylight::start_keyless_heartbeat`]. Dropping it signals
/// the thread and joins it, so the cadence cannot outlive the scope that asked
/// for it. Holds no reference to the client.
pub struct KeylessHeartbeat {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for KeylessHeartbeat {
    fn drop(&mut self) {
        {
            let (lock, cv) = &*self.stop;
            let mut stopped = lock.lock().unwrap_or_else(|e| e.into_inner());
            *stopped = true;
            cv.notify_all();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The server's license-key normalization: strip whitespace and dashes, then
/// uppercase. Mirrors the Worker's `normalizeKey`, which is what every stored
/// `normalizedKey` was built with — a key typed with spaces or in lowercase has
/// to resolve to the same license.
fn normalize_key(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(char::to_uppercase)
        .collect()
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
