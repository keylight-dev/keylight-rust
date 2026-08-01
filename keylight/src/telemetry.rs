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
