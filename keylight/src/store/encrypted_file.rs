use super::{device::{DeviceIdentity, SystemDeviceIdentity}, LicenseStore};
use crate::{KeylightError, Result};
use chacha20poly1305::{aead::{Aead, KeyInit}, ChaCha20Poly1305, Key, Nonce};
use std::path::PathBuf;

pub struct EncryptedFileStore { dir: PathBuf, key: Key }

impl EncryptedFileStore {
    /// Default store under the OS config dir, keyed to this device.
    pub fn new(namespace: &str) -> Result<Self> {
        Self::with_device(namespace, &SystemDeviceIdentity)
    }
    pub fn with_device(namespace: &str, device: &dyn DeviceIdentity) -> Result<Self> {
        let base = directories::ProjectDirs::from("dev", "keylight", "keylight")
            .map(|p| p.data_dir().to_path_buf())
            .ok_or_else(|| KeylightError::Storage("no config dir".into()))?;
        let dir = base.join(namespace);
        std::fs::create_dir_all(&dir).map_err(|e| KeylightError::Storage(e.to_string()))?;
        // Derive a 32-byte key from the device id (BLAKE3 keyed by a fixed domain).
        let derived = blake3::derive_key("keylight-store-v1", device.stable_id().as_bytes());
        Ok(Self { dir, key: *Key::from_slice(&derived) })
    }
    pub fn at_dir(dir: PathBuf, device: &dyn DeviceIdentity) -> Result<Self> {
        std::fs::create_dir_all(&dir).map_err(|e| KeylightError::Storage(e.to_string()))?;
        let derived = blake3::derive_key("keylight-store-v1", device.stable_id().as_bytes());
        Ok(Self { dir, key: *Key::from_slice(&derived) })
    }
    fn path(&self, account: &str) -> PathBuf { self.dir.join(format!("{account}.bin")) }
}

impl LicenseStore for EncryptedFileStore {
    fn get(&self, account: &str) -> Option<Vec<u8>> {
        let blob = std::fs::read(self.path(account)).ok()?;
        if blob.len() < 12 { return None; }
        let (nonce, ct) = blob.split_at(12);
        let cipher = ChaCha20Poly1305::new(&self.key);
        cipher.decrypt(Nonce::from_slice(nonce), ct).ok()
    }
    fn set(&self, account: &str, value: &[u8]) -> Result<()> {
        use rand::RngCore;
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let cipher = ChaCha20Poly1305::new(&self.key);
        let ct = cipher.encrypt(Nonce::from_slice(&nonce_bytes), value)
            .map_err(|_| KeylightError::Storage("encrypt failed".into()))?;
        let mut out = nonce_bytes.to_vec();
        out.extend_from_slice(&ct);
        let tmp = self.path(&format!("{account}.tmp"));
        std::fs::write(&tmp, &out).map_err(|e| KeylightError::Storage(e.to_string()))?;
        std::fs::rename(&tmp, self.path(account)).map_err(|e| KeylightError::Storage(e.to_string()))
    }
    fn delete(&self, account: &str) -> Result<()> {
        match std::fs::remove_file(self.path(account)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(KeylightError::Storage(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::device::FixedDeviceIdentity;
    #[test]
    fn round_trips_encrypted() {
        let tmp = std::env::temp_dir().join(format!("kl-test-{}", super::super::device::FixedDeviceIdentity("x".into()).stable_id()));
        let dev = FixedDeviceIdentity("device-123".into());
        let store = EncryptedFileStore::at_dir(tmp.clone(), &dev).unwrap();
        store.set_string(super::super::account::LICENSE_KEY, "NOTES-PRO0-0000-0001").unwrap();
        assert_eq!(store.get_string(super::super::account::LICENSE_KEY).as_deref(), Some("NOTES-PRO0-0000-0001"));
        // On-disk bytes must not contain the plaintext.
        let raw = std::fs::read(tmp.join("license_key.bin")).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("NOTES-PRO0"));
        store.delete(super::super::account::LICENSE_KEY).unwrap();
        assert!(store.get(super::super::account::LICENSE_KEY).is_none());
    }
    #[test]
    fn wrong_device_cannot_decrypt() {
        let tmp = std::env::temp_dir().join("kl-test-wrongdev");
        EncryptedFileStore::at_dir(tmp.clone(), &FixedDeviceIdentity("dev-a".into())).unwrap()
            .set_string(super::super::account::LEASE, "secret").unwrap();
        let other = EncryptedFileStore::at_dir(tmp, &FixedDeviceIdentity("dev-b".into())).unwrap();
        assert!(other.get(super::super::account::LEASE).is_none());
    }
}
