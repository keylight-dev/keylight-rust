use crate::http::{Transport, TransportOutcome, ureq_transport::UreqTransport};
use crate::http::retry::{decide, RetryDecision, MAX_ATTEMPTS, backoff_ms, clamp_sleep_ms};
use crate::store::{account, LicenseStore, encrypted_file::EncryptedFileStore};
use crate::{KeylightConfig, KeylightError, Lease, Result, verify_lease, telemetry};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ActivationResult { pub activated: bool, pub instance_id: Option<String>, pub lease: Option<Lease>, pub license_expires_at: Option<i64>, pub error: Option<String> }
#[derive(Debug, Clone)]
pub struct ValidationResult { pub valid: bool, pub lease: Option<Lease>, pub license_expires_at: Option<i64>, pub error: Option<String> }

#[derive(Deserialize)]
struct ActivateResp { activated: bool, instance_id: Option<String>, license_expires_at: Option<i64>, lease: Option<Lease>, error: Option<String> }
#[derive(Deserialize)]
struct ValidateResp { valid: bool, license_expires_at: Option<i64>, lease: Option<Lease>, error: Option<String> }
#[derive(Deserialize)]
struct ErrorResp { error: Option<String> }

pub struct Keylight { config: KeylightConfig, store: Arc<dyn LicenseStore>, transport: Arc<dyn Transport> }

impl Keylight {
    /// Construct with the default encrypted-file store + ureq transport.
    pub fn new(config: KeylightConfig) -> Result<Self> {
        let ns = format!("{}-{}", config.tenant_id, config.product_id);
        Ok(Self { store: Arc::new(EncryptedFileStore::new(&ns)?), transport: Arc::new(UreqTransport::default()), config })
    }
    /// Construct with custom store + transport (tests, alternate backends).
    pub fn with_parts(config: KeylightConfig, store: Arc<dyn LicenseStore>, transport: Arc<dyn Transport>) -> Self {
        Self { config, store, transport }
    }

    fn request_id() -> String {
        use rand::Rng;
        let n: u32 = rand::thread_rng().gen();
        format!("{n:08x}")
    }
    fn headers(&self) -> Vec<(String, String)> {
        let mut h = vec![
            ("Content-Type".into(), "application/json".into()),
            ("X-Keylight-Request-Id".into(), Self::request_id()),
        ];
        if let Some(k) = &self.config.sdk_key { h.push(("X-Keylight-SDK-Key".into(), k.clone())); }
        h
    }
    fn body_with_telemetry(&self, mut map: serde_json::Map<String, serde_json::Value>) -> String {
        telemetry::apply(&mut map, self.config.app_version.as_deref());
        serde_json::Value::Object(map).to_string()
    }

    /// POST with retry/backoff. `decodable_4xx` lets a caller opt a 4xx body in (validate's 422).
    fn post(&self, path: &str, body: &str, decodable_4xx: &[u16]) -> Result<(u16, String)> {
        let url = format!("{}/{}/{}/{}", self.config.base_url, self.config.tenant_id, self.config.product_id, path);
        let headers = self.headers();
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match self.transport.post_json(&url, &headers, body) {
                TransportOutcome::Response(r) => {
                    if r.status == 200 || decodable_4xx.contains(&r.status) { return Ok((r.status, r.body)); }
                    match decide(r.status, attempt, r.retry_after) {
                        RetryDecision::RetryAfter(ms) => { std::thread::sleep(std::time::Duration::from_millis(ms)); continue; }
                        RetryDecision::Stop => {
                            if r.status == 429 { return Err(KeylightError::RateLimited { retry_after: r.retry_after.unwrap_or(0) }); }
                            if (500..=599).contains(&r.status) || r.status == 408 { return Err(KeylightError::ServerError { status: r.status }); }
                            let msg = serde_json::from_str::<ErrorResp>(&r.body).ok().and_then(|e| e.error).unwrap_or_default();
                            return Err(KeylightError::ClientError { status: r.status, message: msg });
                        }
                    }
                }
                TransportOutcome::Transient(_) if attempt < MAX_ATTEMPTS => {
                    std::thread::sleep(std::time::Duration::from_millis(clamp_sleep_ms(backoff_ms(attempt)))); continue;
                }
                TransportOutcome::Transient(e) | TransportOutcome::Terminal(e) => return Err(KeylightError::NetworkFailure(e)),
                TransportOutcome::Timeout => return Err(KeylightError::Timeout),
            }
        }
    }

    fn now() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) }

    fn verify_or_reject(&self, lease: &Lease) -> Result<()> {
        let r = verify_lease(lease, &self.config.trusted_keys, Self::now(), crate::SKEW_SECONDS);
        if r.kid_known && r.signature_valid { Ok(()) } else { Err(KeylightError::LeaseVerificationFailed) }
    }

    pub fn activate(&self, key: &str) -> Result<ActivationResult> {
        if !self.config.validate_key_format(key) {
            return Ok(ActivationResult { activated: false, instance_id: None, lease: None, license_expires_at: None, error: Some("Invalid license key format".into()) });
        }
        let machine = hostname_or("device");
        let mut map = serde_json::Map::new();
        map.insert("license_key".into(), key.into());
        map.insert("instance_name".into(), machine.into());
        if let Some(ft) = self.store.get_string(account::FREE_TIER_INSTANCE_ID) { map.insert("free_tier_instance_id".into(), ft.into()); }
        let body = self.body_with_telemetry(map);

        let (status, text) = match self.post("activate", &body, &[]) {
            Ok(v) => v,
            Err(KeylightError::ClientError { status, message }) => {
                return Ok(ActivationResult { activated: false, instance_id: None, lease: None, license_expires_at: None, error: Some(if message.is_empty() { format!("Activation failed (HTTP {status})") } else { message }) });
            }
            Err(e) => return Err(e),
        };
        let _ = status;
        let resp: ActivateResp = serde_json::from_str(&text).map_err(|_| KeylightError::InvalidResponse)?;
        if !resp.activated { return Ok(ActivationResult { activated: false, instance_id: None, lease: None, license_expires_at: None, error: resp.error.or(Some("Activation failed".into())) }); }
        if let Some(lease) = &resp.lease { self.verify_or_reject(lease)?; }

        self.store.set_string(account::LICENSE_KEY, key)?;
        if let Some(id) = &resp.instance_id { self.store.set_string(account::INSTANCE_ID, id)?; }
        if let Some(lease) = &resp.lease { self.store.set_string(account::LEASE, &serde_json::to_string(lease).unwrap())?; }
        self.save_expiry(resp.license_expires_at)?;
        self.touch_last_seen()?; self.touch_validated_online()?;
        Ok(ActivationResult { activated: true, instance_id: resp.instance_id, lease: resp.lease, license_expires_at: resp.license_expires_at, error: None })
    }

    pub fn validate(&self) -> Result<ValidationResult> {
        let key = self.store.get_string(account::LICENSE_KEY).ok_or(KeylightError::NoStoredLicense)?;
        let instance = self.store.get_string(account::INSTANCE_ID).ok_or(KeylightError::NoStoredLicense)?;
        let mut map = serde_json::Map::new();
        map.insert("license_key".into(), key.into());
        map.insert("instance_id".into(), instance.into());
        let body = self.body_with_telemetry(map);

        let (_status, text) = match self.post("validate", &body, &[422]) {
            Ok(v) => v,
            Err(KeylightError::ClientError { status, message }) => return Ok(ValidationResult { valid: false, lease: None, license_expires_at: None, error: Some(if message.is_empty() { format!("Validation failed (HTTP {status})") } else { message }) }),
            Err(e) => return Err(e),
        };
        let resp: ValidateResp = serde_json::from_str(&text).map_err(|_| KeylightError::InvalidResponse)?;
        if let Some(lease) = &resp.lease { self.verify_or_reject(lease)?; }
        if !resp.valid {
            // Preserve fallback/expired lease so the manager can resolve .limited/.expired.
            return Ok(ValidationResult { valid: false, lease: resp.lease, license_expires_at: resp.license_expires_at, error: resp.error });
        }
        if let Some(lease) = &resp.lease { self.store.set_string(account::LEASE, &serde_json::to_string(lease).unwrap())?; }
        self.save_expiry(resp.license_expires_at)?;
        self.touch_last_seen()?; self.touch_validated_online()?;
        Ok(ValidationResult { valid: true, lease: resp.lease, license_expires_at: resp.license_expires_at, error: None })
    }

    pub fn deactivate(&self) -> Result<()> {
        let key = self.store.get_string(account::LICENSE_KEY);
        let instance = self.store.get_string(account::INSTANCE_ID);
        let mut net_err = None;
        if let (Some(k), Some(i)) = (key, instance) {
            let mut map = serde_json::Map::new();
            map.insert("license_key".into(), k.into());
            map.insert("instance_id".into(), i.into());
            let body = serde_json::Value::Object(map).to_string();
            if let Err(e) = self.post("deactivate", &body, &[]) { net_err = Some(e); }
        }
        for a in [account::LICENSE_KEY, account::INSTANCE_ID, account::LEASE, account::LICENSE_EXPIRES_AT, account::LAST_VALIDATED_ONLINE, account::LAST_SEEN] {
            self.store.delete(a)?;
        }
        match net_err { Some(e) => Err(e), None => Ok(()) }
    }

    pub fn cached_lease(&self) -> Option<Lease> {
        if let Some(max_days) = self.config.max_offline_days {
            let last = self.store.get_string(account::LAST_VALIDATED_ONLINE).and_then(|s| s.parse::<i64>().ok())?;
            if Self::now() - last > (max_days as i64) * 86400 { return None; }
        }
        let lease: Lease = serde_json::from_str(&self.store.get_string(account::LEASE)?).ok()?;
        let r = verify_lease(&lease, &self.config.trusted_keys, Self::now(), crate::SKEW_SECONDS);
        if r.kid_known && r.signature_valid && !r.expired { Some(lease) } else { None }
    }

    pub fn has_entitlement(&self, feature: &str) -> bool {
        self.cached_lease().map(|l| l.entitlements.iter().any(|e| e == feature)).unwrap_or(false)
    }
    pub fn has_stored_license(&self) -> bool { self.store.get_string(account::LICENSE_KEY).is_some() }
    pub fn cached_license_key(&self) -> Option<String> { self.store.get_string(account::LICENSE_KEY) }

    fn save_expiry(&self, e: Option<i64>) -> Result<()> {
        match e { Some(v) => self.store.set_string(account::LICENSE_EXPIRES_AT, &v.to_string()), None => self.store.delete(account::LICENSE_EXPIRES_AT) }
    }
    fn touch_last_seen(&self) -> Result<()> { self.store.set_string(account::LAST_SEEN, &Self::now().to_string()) }
    fn touch_validated_online(&self) -> Result<()> { self.store.set_string(account::LAST_VALIDATED_ONLINE, &Self::now().to_string()) }
}

fn hostname_or(default: &str) -> String { std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| default.to_string()) }
