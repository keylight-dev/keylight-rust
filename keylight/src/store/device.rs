/// Stable per-device identifier (parity with Swift SystemDeviceIdentity).
pub trait DeviceIdentity: Send + Sync {
    fn stable_id(&self) -> String;
}

pub struct SystemDeviceIdentity;
impl DeviceIdentity for SystemDeviceIdentity {
    fn stable_id(&self) -> String {
        read_machine_id().unwrap_or_else(persisted_fallback_id)
    }
}

/// Caller-provided fixed id (parity with Swift FixedDeviceIdentity), for tests/CI.
pub struct FixedDeviceIdentity(pub String);
impl DeviceIdentity for FixedDeviceIdentity {
    fn stable_id(&self) -> String {
        self.0.clone()
    }
}

#[cfg(target_os = "linux")]
fn read_machine_id() -> Option<String> {
    std::fs::read_to_string("/etc/machine-id")
        .ok()
        .or_else(|| std::fs::read_to_string("/var/lib/dbus/machine-id").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
#[cfg(target_os = "macos")]
fn read_machine_id() -> Option<String> {
    let out = std::process::Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find(|l| l.contains("IOPlatformUUID"))
        .and_then(|l| l.split('"').nth(3))
        .map(|s| s.to_string())
}
#[cfg(target_os = "windows")]
fn read_machine_id() -> Option<String> {
    let out = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace()
        .last()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn read_machine_id() -> Option<String> {
    None
}

/// Last resort: a random UUID persisted in the config dir, so the id is stable per install.
fn persisted_fallback_id() -> String {
    use std::io::Write;
    let dir = directories::ProjectDirs::from("dev", "keylight", "keylight")
        .map(|p| p.config_dir().to_path_buf())
        .unwrap_or_else(std::env::temp_dir);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("device-id");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let t = existing.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    let id = uuid_v4();
    if let Ok(mut f) = std::fs::File::create(&path) {
        let _ = f.write_all(id.as_bytes());
    }
    id
}

/// Public wrapper around the internal UUIDv4 generator (used by the client for free-tier ids).
pub fn uuid_v4_pub() -> String {
    uuid_v4()
}

/// Tiny UUIDv4 from `rand` (avoids a uuid dependency).
fn uuid_v4() -> String {
    use rand::Rng;
    let b: [u8; 16] = rand::thread_rng().gen();
    let mut b = b;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!("{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],b[1],b[2],b[3],b[4],b[5],b[6],b[7],b[8],b[9],b[10],b[11],b[12],b[13],b[14],b[15])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_identity_returns_value() {
        assert_eq!(FixedDeviceIdentity("abc".into()).stable_id(), "abc");
    }
    #[test]
    fn system_identity_is_nonempty_and_stable() {
        let a = SystemDeviceIdentity.stable_id();
        assert!(!a.is_empty());
        assert_eq!(a, SystemDeviceIdentity.stable_id());
    }
    #[test]
    fn uuid_v4_has_version_nibble() {
        assert_eq!(uuid_v4().chars().nth(14).unwrap(), '4');
    }
}
