use keylight::state::{KeylessState, LicenseState};
use keylight::{Keylight, KeylightConfig};
use std::sync::Arc;
use std::time::Duration;
use tauri::{
    Manager, Runtime, State,
    plugin::{Builder, TauriPlugin},
};

struct KeylightState(Arc<Keylight>);

#[tauri::command]
fn activate(state: State<'_, KeylightState>, key: String) -> Result<bool, String> {
    state
        .0
        .activate(&key)
        .map(|r| r.activated)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn validate(state: State<'_, KeylightState>) -> Result<bool, String> {
    state
        .0
        .validate()
        .map(|r| r.valid)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn has_entitlement(state: State<'_, KeylightState>, feature: String) -> bool {
    state.0.has_entitlement(&feature)
}

/// Validate the stored license against the server (no staleness gate); call on app launch.
#[tauri::command]
fn check_on_launch(state: State<'_, KeylightState>) -> Result<(), String> {
    state.0.check_on_launch().map_err(|e| e.to_string())
}

/// Re-validate only if the debounce/staleness policy says it's time.
/// Returns the validation result (`true`/`false`) when a validation ran, `null` otherwise.
#[tauri::command]
fn refresh_if_needed(state: State<'_, KeylightState>) -> Result<Option<bool>, String> {
    state
        .0
        .refresh_if_needed()
        .map(|r| r.map(|v| v.valid))
        .map_err(|e| e.to_string())
}

/// Send the anonymous keyless beacon. `keylessState` is the wire value:
/// `"trial"`, `"free_tier"`, or `"expired"`.
#[tauri::command]
fn report_keyless_state(state: State<'_, KeylightState>, keyless_state: String) -> Result<(), String> {
    let ks = parse_keyless_state(&keyless_state)
        .ok_or_else(|| format!("unknown keyless state: {keyless_state}"))?;
    state.0.report_keyless_state(ks);
    Ok(())
}

fn parse_keyless_state(s: &str) -> Option<KeylessState> {
    match s {
        "trial" => Some(KeylessState::Trial),
        "free_tier" => Some(KeylessState::FreeTier),
        "expired" => Some(KeylessState::Expired),
        _ => None,
    }
}

/// Map the resolved license state to the keyless beacon state, if one applies.
fn keyless_state_for(state: &LicenseState) -> Option<KeylessState> {
    match state {
        LicenseState::Trial { .. } => Some(KeylessState::Trial),
        LicenseState::FreeTier => Some(KeylessState::FreeTier),
        LicenseState::Expired => Some(KeylessState::Expired),
        _ => None,
    }
}

/// Options for the plugin's optional built-in heartbeat scheduler.
#[derive(Clone, Debug)]
pub struct HeartbeatOptions {
    /// Spawn a background task that periodically calls `refresh_if_needed` and
    /// `report_keyless_state`. Off by default (the app drives refreshes itself).
    pub enabled: bool,
    /// Interval between heartbeats. Default 6 hours (matches the SDK's staleness window).
    pub interval: Duration,
}

impl Default for HeartbeatOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            interval: Duration::from_secs(6 * 60 * 60),
        }
    }
}

/// Initialize with a prebuilt config (the host app supplies tenant/product/keys).
/// The built-in heartbeat scheduler is off; use [`init_with_heartbeat`] to enable it.
pub fn init<R: Runtime>(config: KeylightConfig) -> TauriPlugin<R> {
    init_with_heartbeat(config, HeartbeatOptions::default())
}

/// Initialize with a prebuilt config and heartbeat scheduler options. When
/// `heartbeat.enabled`, a background task calls `refresh_if_needed` (debounced
/// in the SDK) and, when the app is in a keyless state, `report_keyless_state`
/// (debounced 24h in the SDK) every `heartbeat.interval`.
pub fn init_with_heartbeat<R: Runtime>(
    config: KeylightConfig,
    heartbeat: HeartbeatOptions,
) -> TauriPlugin<R> {
    Builder::new("keylight")
        .invoke_handler(tauri::generate_handler![
            activate,
            validate,
            has_entitlement,
            check_on_launch,
            refresh_if_needed,
            report_keyless_state
        ])
        .setup(move |app, _api| {
            let kl = Arc::new(Keylight::new(config.clone())?);
            app.manage(KeylightState(kl.clone()));
            if heartbeat.enabled {
                let interval = heartbeat.interval.max(Duration::from_secs(60));
                // The SDK is synchronous/blocking (ureq), so the scheduler is a
                // plain thread rather than an async task on the tauri runtime.
                std::thread::spawn(move || {
                    loop {
                        std::thread::sleep(interval);
                        let _ = kl.refresh_if_needed();
                        if let Some(ks) = keyless_state_for(&kl.state()) {
                            kl.report_keyless_state(ks);
                        }
                    }
                });
            }
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wire_keyless_states() {
        assert_eq!(parse_keyless_state("trial"), Some(KeylessState::Trial));
        assert_eq!(
            parse_keyless_state("free_tier"),
            Some(KeylessState::FreeTier)
        );
        assert_eq!(parse_keyless_state("expired"), Some(KeylessState::Expired));
        assert_eq!(parse_keyless_state("bogus"), None);
    }

    #[test]
    fn maps_license_state_to_keyless_state() {
        assert_eq!(
            keyless_state_for(&LicenseState::Trial { days_left: 3 }),
            Some(KeylessState::Trial)
        );
        assert_eq!(
            keyless_state_for(&LicenseState::FreeTier),
            Some(KeylessState::FreeTier)
        );
        assert_eq!(
            keyless_state_for(&LicenseState::Expired),
            Some(KeylessState::Expired)
        );
        assert_eq!(keyless_state_for(&LicenseState::Licensed), None);
        assert_eq!(keyless_state_for(&LicenseState::Limited), None);
        assert_eq!(keyless_state_for(&LicenseState::Invalid), None);
    }

    #[test]
    fn heartbeat_defaults_off_six_hours() {
        let o = HeartbeatOptions::default();
        assert!(!o.enabled);
        assert_eq!(o.interval, Duration::from_secs(21600));
    }
}
