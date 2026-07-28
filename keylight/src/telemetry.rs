//! SDK/platform/app-version telemetry fields attached to API requests.

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
// `sdk_version` ≤ 64 chars, `platform` ≤ 32. An over-long value fails the whole
// request with a 400 — it is not dropped silently — and `app_version` comes
// from the host app, so clamp before sending (parity with the JS SDK).
const VERSION_MAX: usize = 64;
const PLATFORM_MAX: usize = 32;

/// Truncate to at most `max` characters, never splitting a UTF-8 sequence.
fn clamp(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s,
    }
}

/// Inject telemetry fields into a request body map, clamped to the server caps.
pub fn apply(map: &mut serde_json::Map<String, serde_json::Value>, app_version: Option<&str>) {
    map.insert(
        "sdk_version".into(),
        clamp(sdk_version(), VERSION_MAX).into(),
    );
    map.insert("platform".into(), clamp(platform(), PLATFORM_MAX).into());
    if let Some(av) = app_version {
        map.insert("app_version".into(), clamp(av, VERSION_MAX).into());
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

    /// The compile-time values are already well inside the caps — a regression
    /// here (a long platform string, say) would 400 every request.
    #[test]
    fn built_in_values_are_within_the_caps() {
        assert!(sdk_version().len() <= VERSION_MAX);
        assert!(platform().len() <= PLATFORM_MAX);
    }
}
