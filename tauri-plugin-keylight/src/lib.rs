use keylight::state::{KeylessState, keyless_state_for};
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

/// Force a re-validation on active use (app foreground, window focus). Debounced
/// to 60s in the SDK. Wire this to a foreground/focus hook (see README) so a
/// dashboard revoke lands within minutes instead of waiting for the normal
/// refresh cadence or a relaunch.
///
/// Returns `true`/`false` when a validation ran, `null` when suppressed by the
/// debounce window, no license is stored, or the call was transient (network
/// error) and state was left untouched.
#[tauri::command]
fn active_revalidate(state: State<'_, KeylightState>) -> Option<bool> {
    state.0.active_revalidate().map(|r| r.valid)
}

/// Briefly poll-revalidate after a customer completes an upgrade, so new
/// entitlements (or a mid-flight rejection) show up in the running app without
/// waiting for the normal refresh cadence. Re-validates every
/// `poll_interval_secs` (default 2.0, clamped to a 100ms floor in the SDK)
/// until `timeout_secs` (default 30.0) elapses or a change is observed.
///
/// **Blocking.** Tauri dispatches synchronous commands off the main thread, so
/// this is safe to call directly from the frontend, but it can take up to
/// `timeout_secs` to resolve.
///
/// Returns `true` as soon as a change is detected, `false` on timeout or when
/// no license is stored. Errors only on invalid (negative/NaN/infinite)
/// duration input.
#[tauri::command]
fn refresh_after_upgrade(
    state: State<'_, KeylightState>,
    timeout_secs: Option<f64>,
    poll_interval_secs: Option<f64>,
) -> Result<bool, String> {
    let timeout = secs_to_duration(timeout_secs.unwrap_or(30.0))?;
    let poll_interval = secs_to_duration(poll_interval_secs.unwrap_or(2.0))?;
    Ok(state.0.refresh_after_upgrade(timeout, poll_interval))
}

fn secs_to_duration(secs: f64) -> Result<Duration, String> {
    // try_from_secs_f64 rejects negative, NaN, infinite *and* overflowing values;
    // the plain from_secs_f64 would panic on a finite but too-large input (e.g. 1e300).
    Duration::try_from_secs_f64(secs).map_err(|_| format!("invalid duration: {secs} seconds"))
}

/// Send the anonymous keyless beacon. `keylessState` is the wire value:
/// `"trial"`, `"free_tier"`, or `"expired"`.
#[tauri::command]
fn report_keyless_state(
    state: State<'_, KeylightState>,
    keyless_state: String,
) -> Result<(), String> {
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

/// Options for the plugin's built-in heartbeat scheduler.
#[derive(Clone, Debug)]
pub struct HeartbeatOptions {
    /// Spawn a background thread that periodically re-reports the anonymous
    /// keyless beacon. **On by default.** A Tauri app is typically a tray app
    /// that stays resident for days; without a heartbeat it beacons once per
    /// process launch and the dashboard shows a live install as last seen on
    /// the day it was installed. The beacon carries no PII and the SDK debounces
    /// it to one request per 24h, so there is nothing here for a host to weigh.
    pub enabled: bool,
    /// Also call `refresh_if_needed` on each tick. **Off by default** — that is
    /// a licensing decision (and a network call on a licensed device) the host
    /// app owns, and it is deliberately not bundled into the liveness beacon.
    pub revalidate: bool,
    /// Interval between heartbeats. Default 6 hours, matching the Worker's
    /// server-side gate on keyless writes: a tick never arrives before the
    /// server is willing to record it.
    pub interval: Duration,
}

impl Default for HeartbeatOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            revalidate: false,
            interval: Duration::from_secs(6 * 60 * 60),
        }
    }
}

/// Initialize with a prebuilt config (the host app supplies tenant/product/keys).
/// The keyless beacon heartbeat runs by default; use [`init_with_heartbeat`] to
/// change its interval, add license revalidation, or turn it off.
pub fn init<R: Runtime>(config: KeylightConfig) -> TauriPlugin<R> {
    init_with_heartbeat(config, HeartbeatOptions::default())
}

/// Initialize with a prebuilt config and heartbeat scheduler options. When
/// `heartbeat.enabled`, a background thread calls `report_keyless_state`
/// (debounced 24h in the SDK) every `heartbeat.interval` for as long as the app
/// is in a keyless state — and, only when `heartbeat.revalidate` is also set,
/// `refresh_if_needed` (debounced in the SDK) on the same tick.
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
            active_revalidate,
            refresh_after_upgrade,
            report_keyless_state
        ])
        .setup(move |app, _api| {
            let kl = Arc::new(Keylight::new(config.clone())?);
            app.manage(KeylightState(kl.clone()));
            if heartbeat.enabled {
                let interval = heartbeat.interval.max(Duration::from_secs(60));
                // The SDK is synchronous/blocking (ureq), so the scheduler is a
                // plain thread rather than an async task on the tauri runtime.
                let revalidate = heartbeat.revalidate;
                std::thread::spawn(move || {
                    loop {
                        std::thread::sleep(interval);
                        if revalidate {
                            let _ = kl.refresh_if_needed();
                        }
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

    // A tray app is the normal shape of a Tauri app, and it stays resident for
    // days. Leaving the beacon to the host meant every such app reported itself
    // once per launch and then looked dead to the dashboard: last_seen never
    // moved past first_seen. The beacon is anonymous and the SDK debounces it
    // to one request per 24h, so there is nothing for a host to opt into.
    #[test]
    fn heartbeat_beacons_by_default_without_revalidating() {
        let o = HeartbeatOptions::default();
        assert!(
            o.enabled,
            "a resident keyless app must keep beaconing without the host opting in"
        );
        assert!(
            !o.revalidate,
            "re-validating a license is a network call the host app still owns"
        );
        assert_eq!(o.interval, Duration::from_secs(21600));
    }

    #[test]
    fn secs_to_duration_converts_finite_nonnegative() {
        assert_eq!(secs_to_duration(30.0), Ok(Duration::from_secs_f64(30.0)));
        assert_eq!(secs_to_duration(0.0), Ok(Duration::ZERO));
    }

    #[test]
    fn secs_to_duration_rejects_negative_nan_infinite_overflow() {
        assert!(secs_to_duration(-1.0).is_err());
        assert!(secs_to_duration(f64::NAN).is_err());
        assert!(secs_to_duration(f64::INFINITY).is_err());
        // finite and non-negative, but overflows Duration — would panic under from_secs_f64.
        assert!(secs_to_duration(1e300).is_err());
    }
}
