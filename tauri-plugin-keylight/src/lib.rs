use keylight::{Keylight, KeylightConfig};
use std::sync::Arc;
use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime, State,
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

/// Initialize with a prebuilt config (the host app supplies tenant/product/keys).
pub fn init<R: Runtime>(config: KeylightConfig) -> TauriPlugin<R> {
    Builder::new("keylight")
        .invoke_handler(tauri::generate_handler![
            activate,
            validate,
            has_entitlement
        ])
        .setup(move |app, _api| {
            let kl = Keylight::new(config.clone())?;
            app.manage(KeylightState(Arc::new(kl)));
            Ok(())
        })
        .build()
}
