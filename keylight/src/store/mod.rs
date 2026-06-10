//! License storage: the [`LicenseStore`] trait, stable account keys, per-OS device
//! identity, and the default device-bound encrypted file store.

pub mod device;
pub mod encrypted_file;

/// Stable account keys for stored values (parity with Swift StorageAccount).
pub mod account {
    pub const LICENSE_KEY: &str = "license_key";
    pub const INSTANCE_ID: &str = "instance_id";
    pub const LEASE: &str = "lease";
    pub const LICENSE_EXPIRES_AT: &str = "license_expires_at";
    pub const LAST_SEEN: &str = "last_seen";
    pub const LAST_VALIDATED_ONLINE: &str = "last_validated_online";
    pub const TRIAL_START: &str = "trial_start";
    pub const FREE_TIER_INSTANCE_ID: &str = "free_tier_instance_id";
    pub const KEYLESS_LAST_STATE: &str = "keyless_last_state";
    pub const LAST_KEYLESS_PING_AT: &str = "last_keyless_ping_at";
}

/// Opaque per-account byte storage. Default impl is `EncryptedFileStore`.
pub trait LicenseStore: Send + Sync {
    fn get(&self, account: &str) -> Option<Vec<u8>>;
    fn set(&self, account: &str, value: &[u8]) -> crate::Result<()>;
    fn delete(&self, account: &str) -> crate::Result<()>;

    fn get_string(&self, account: &str) -> Option<String> {
        self.get(account).and_then(|b| String::from_utf8(b).ok())
    }
    fn set_string(&self, account: &str, value: &str) -> crate::Result<()> {
        self.set(account, value.as_bytes())
    }
}
