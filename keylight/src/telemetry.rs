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

/// Inject telemetry fields into a request body map.
pub fn apply(map: &mut serde_json::Map<String, serde_json::Value>, app_version: Option<&str>) {
    map.insert("sdk_version".into(), sdk_version().into());
    map.insert("platform".into(), platform().into());
    if let Some(av) = app_version {
        map.insert("app_version".into(), av.into());
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
}
