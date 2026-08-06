//! SDK/platform/app-version telemetry fields attached to API requests.

/// Identifies this SDK to the backend, sent as `sdk`.
///
/// `platform` used to identify the SDK implicitly, because each SDK had its own
/// vocabulary for it. That signal is gone: Rust, C++ and C# all send the same
/// canonical `macos`/`windows`/`linux` tokens, so the server could not tell them
/// apart and labelled all three "Rust". Saying which SDK this is explicitly is
/// the fix.
pub const SDK_ID: &str = "rust";

/// SDK version baked at compile time.
pub fn sdk_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Compile-time platform string (parity with Swift currentPlatform()).
pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "unknown"
    }
}

// Server-side caps (activate / validate / keyless): `app_version` and
// `sdk_version` ≤ 64, `platform` ≤ 32. An over-long value fails the whole
// request with a 400 — it is not dropped silently — and `app_version` comes
// from the host app, so clamp before sending (parity with the JS SDK).
const VERSION_MAX: usize = 64;
const PLATFORM_MAX: usize = 32;
const SDK_ID_MAX: usize = 16;

/// Truncate to at most `max` UTF-16 code units, never splitting a character.
///
/// The cap is enforced server-side by zod's `z.string().max(n)`, which counts
/// UTF-16 code units — the same thing JS `.length` and .NET `string.Length`
/// count. Counting `char`s here instead would disagree for anything outside the
/// BMP: 64 emoji are 64 Rust `char`s but 128 code units, so a value this
/// function called "clamped" would still 400. Byte length is likewise wrong
/// (UTF-8, not UTF-16).
fn clamp(s: &str, max: usize) -> &str {
    let mut units = 0;
    for (byte_idx, ch) in s.char_indices() {
        let next = units + ch.len_utf16();
        if next > max {
            return &s[..byte_idx];
        }
        units = next;
    }
    s
}

// Phase-3 device dimensions (activate / validate / keyless — NOT deactivate).
// `os_version` is validated server-side against `\d+(\.\d+)*` and a 32-char
// cap: anything else is nulled, and an over-long value 400s the whole request
// (zod `.max(32)`), so both shapes are enforced here before sending.
const OS_VERSION_MAX: usize = 32;

/// Canonical CPU-architecture token for this build target.
///
/// The server allow-lists exactly two spellings — `arm64` and `x86_64` — and
/// canonicalizes aliases (`aarch64` → `arm64`), but we send the canonical
/// spelling ourselves. Targets outside the vocabulary (32-bit, exotic ISAs)
/// return `None` and the field is omitted: absent reads better server-side
/// than a long tail of one-off buckets it would drop anyway.
pub(crate) fn arch() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "aarch64" => Some("arm64"),
        "x86_64" => Some("x86_64"),
        _ => None,
    }
}

/// Dotted-numeric OS version ("15.5", "6.8.0"), read once per process.
///
/// `None` when the platform read fails or yields nothing dotted-numeric — the
/// field is omitted rather than shipping junk bytes the server would null.
pub(crate) fn os_version() -> Option<&'static str> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Option<String>> = OnceLock::new();
    CACHE
        .get_or_init(|| read_os_version_raw().as_deref().and_then(dotted_numeric))
        .as_deref()
}

/// Extract the first dotted-numeric run (`\d+(\.\d+)*`) from a raw OS string.
///
/// Handles every shape the per-OS reads produce: `sw_vers` is already clean
/// ("15.5"), a Linux kernel release carries a suffix ("6.8.0-45-generic"),
/// and Windows `ver` wraps the number in prose ("Microsoft Windows [Version
/// 10.0.22631.3737]"). A trailing dot is dropped rather than sent; a run over
/// the server's 32-char cap is rejected rather than truncated — truncation
/// would mint a fake version bucket out of a client bug.
fn dotted_numeric(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let start = bytes.iter().position(u8::is_ascii_digit)?;
    let run = &raw[start..];
    let end = run
        .bytes()
        .position(|b| !b.is_ascii_digit() && b != b'.')
        .unwrap_or(run.len());
    let v = run[..end].trim_end_matches('.');
    let well_formed = v.len() <= OS_VERSION_MAX
        && v.split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    well_formed.then(|| v.to_string())
}

// Per-OS raw reads, in the same style as `store::device::read_machine_id` —
// zero new dependencies, `None` on any failure.
#[cfg(target_os = "macos")]
fn read_os_version_raw() -> Option<String> {
    let out = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
#[cfg(target_os = "linux")]
fn read_os_version_raw() -> Option<String> {
    // Kernel release ("6.8.0-45-generic") — the one version every Linux has;
    // distro versions live in /etc/os-release but aren't comparable across
    // distros, and the kernel is what OS-level behavior actually tracks.
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|s| s.trim().to_string())
}
#[cfg(target_os = "windows")]
fn read_os_version_raw() -> Option<String> {
    // "Microsoft Windows [Version 10.0.22631.3737]" — prose may be localized,
    // but `dotted_numeric` only reads the numeric run so that doesn't matter.
    let out = std::process::Command::new("cmd")
        .args(["/c", "ver"])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn read_os_version_raw() -> Option<String> {
    None
}

/// Inject telemetry fields into a request body map, clamped to the server caps.
pub fn apply(map: &mut serde_json::Map<String, serde_json::Value>, app_version: Option<&str>) {
    map.insert(
        "sdk_version".into(),
        clamp(sdk_version(), VERSION_MAX).into(),
    );
    map.insert("platform".into(), clamp(platform(), PLATFORM_MAX).into());
    map.insert("sdk".into(), clamp(SDK_ID, SDK_ID_MAX).into());
    if let Some(av) = app_version {
        map.insert("app_version".into(), clamp(av, VERSION_MAX).into());
    }
}

/// Inject the Phase-3 device dimensions (`os_version`, `arch`) — activate /
/// validate / keyless only; deactivate identifies a device, it doesn't
/// describe one. `device_class` is NEVER sent from this SDK: the server only
/// honors it from iOS SDKs and derives desktop classes from the OS token
/// itself. Both values are pre-validated against the server's shape rules, so
/// no clamping — a value that doesn't fit is omitted, not truncated.
pub fn apply_device(map: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(v) = os_version() {
        map.insert("os_version".into(), v.into());
    }
    if let Some(a) = arch() {
        map.insert("arch".into(), a.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_sets_sdk_and_platform_and_optional_app() {
        let mut m = serde_json::Map::new();
        apply(&mut m, Some("1.2.3"));
        assert_eq!(m["sdk_version"], serde_json::json!(sdk_version()));
        assert!(m.contains_key("platform"));
        assert_eq!(m["sdk"], serde_json::json!("rust"));
        assert_eq!(m["app_version"], serde_json::json!("1.2.3"));

        let mut m2 = serde_json::Map::new();
        apply(&mut m2, None);
        assert!(!m2.contains_key("app_version"));
    }

    /// An over-long `app_version` is a hard 400 on the whole request server-side,
    /// so it must be truncated rather than sent as-is.
    #[test]
    fn apply_clamps_app_version_to_the_server_cap() {
        let mut m = serde_json::Map::new();
        apply(&mut m, Some(&"9".repeat(200)));
        assert_eq!(m["app_version"].as_str().unwrap().len(), VERSION_MAX);
    }

    /// Truncation must not split a multi-byte character (that would panic).
    #[test]
    fn clamp_respects_char_boundaries() {
        let s = "é".repeat(100);
        let out = clamp(&s, VERSION_MAX);
        assert_eq!(out.chars().count(), VERSION_MAX);
        assert!(s.starts_with(out));
    }

    /// The cap the server enforces is in UTF-16 code units (zod `.max()`), not
    /// `char`s. Non-BMP characters are 2 units each, so clamping by `char` would
    /// emit 128 units for a 64-cap field and 400 the whole request.
    #[test]
    fn clamp_counts_utf16_code_units_like_the_server() {
        let s = "😀".repeat(100);
        let out = clamp(&s, VERSION_MAX);

        assert_eq!(out.encode_utf16().count(), VERSION_MAX);
        assert_eq!(out.chars().count(), VERSION_MAX / 2);
        assert!(s.starts_with(out));
    }

    /// An odd cap landing mid-pair must round down, never emit a partial one.
    #[test]
    fn clamp_rounds_down_when_the_cap_splits_a_pair() {
        let s = "😀".repeat(10);
        let out = clamp(&s, 5);

        assert_eq!(out.encode_utf16().count(), 4);
        assert_eq!(out.chars().count(), 2);
    }

    /// Mixed BMP/non-BMP: the running count, not a per-char assumption, is what
    /// has to match the server.
    #[test]
    fn clamp_handles_mixed_width_text() {
        let s = format!("{}{}", "a".repeat(10), "😀".repeat(50));
        let out = clamp(&s, VERSION_MAX);

        assert!(out.encode_utf16().count() <= VERSION_MAX);
        assert_eq!(out.encode_utf16().count(), VERSION_MAX);
        assert!(s.starts_with(out));
    }

    /// Every raw shape the per-OS reads can produce must reduce to a clean
    /// dotted-numeric version — or to nothing.
    #[test]
    fn dotted_numeric_extracts_from_all_platform_shapes() {
        assert_eq!(dotted_numeric("15.5"), Some("15.5".into())); // sw_vers
        assert_eq!(dotted_numeric("6.8.0-45-generic"), Some("6.8.0".into())); // kernel
        assert_eq!(
            dotted_numeric("Microsoft Windows [Version 10.0.22631.3737]"),
            Some("10.0.22631.3737".into()) // ver
        );
        assert_eq!(dotted_numeric("14."), Some("14".into())); // trailing dot dropped
    }

    /// Junk never becomes a version bucket: no digits, empty, or over the
    /// server's 32-char cap (which would 400 the whole request) → None.
    #[test]
    fn dotted_numeric_rejects_junk() {
        assert_eq!(dotted_numeric("Sonoma"), None);
        assert_eq!(dotted_numeric(""), None);
        assert_eq!(dotted_numeric(&"1.".repeat(30)), None); // 59 chars > cap
    }

    /// This machine's values must be exactly what the server accepts: arch on
    /// the two-token allow-list, os_version dotted-numeric within the cap.
    #[test]
    fn device_values_match_the_server_vocabulary() {
        if let Some(a) = arch() {
            assert!(a == "arm64" || a == "x86_64");
        }
        if let Some(v) = os_version() {
            assert_eq!(dotted_numeric(v).as_deref(), Some(v));
        }
    }

    /// apply_device sends only what it has — and never device_class.
    #[test]
    fn apply_device_sets_only_present_fields_and_never_device_class() {
        let mut m = serde_json::Map::new();
        apply_device(&mut m);
        assert_eq!(m.contains_key("os_version"), os_version().is_some());
        assert_eq!(m.contains_key("arch"), arch().is_some());
        assert!(!m.contains_key("device_class"));
    }

    /// The compile-time values are already well inside the caps — a regression
    /// here (a long platform string, say) would 400 every request.
    #[test]
    fn built_in_values_are_within_the_caps() {
        assert!(sdk_version().len() <= VERSION_MAX);
        // The server maps this token to a display name; an unrecognised or
        // truncated value falls back to the legacy heuristic, which cannot tell
        // Rust from C++ or C#.
        assert!(SDK_ID.len() <= SDK_ID_MAX);
        assert!(platform().len() <= PLATFORM_MAX);
    }
}
