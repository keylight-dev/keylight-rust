# tauri-plugin-keylight

Tauri v2 plugin for the [Keylight](https://keylight.dev) licensing SDK — activate and validate
license keys and gate features on entitlements from a Tauri desktop app.

This crate is the **Rust side** of the plugin; the matching JavaScript/TypeScript bindings are
published as [`tauri-plugin-keylight-api`](./package.json).

## Install

**Rust (`src-tauri/Cargo.toml`):**

```toml
[dependencies]
tauri-plugin-keylight = { git = "https://github.com/keylight-dev/keylight-rust" }
```

**JavaScript:**

```bash
npm add tauri-plugin-keylight-api
```

## Register the plugin

```rust
// src-tauri/src/main.rs
use keylight::KeylightConfig;

fn main() {
    let cfg = KeylightConfig::builder("your-tenant", "your-product", "sdk_live_…").build();
    tauri::Builder::default()
        .plugin(tauri_plugin_keylight::init(cfg))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

## Permissions

Add the plugin's default permission set to your capability file:

```json
// src-tauri/capabilities/default.json
{
  "permissions": ["keylight:default"]
}
```

`keylight:default` allows `activate`, `validate`, `has_entitlement`, `check_on_launch`,
`refresh_if_needed`, `active_revalidate`, `refresh_after_upgrade`, and `report_keyless_state`. You
can also grant the per-command permissions individually (`keylight:allow-activate`,
`keylight:allow-validate`, `keylight:allow-has-entitlement`, etc.).

## Use from the frontend

```typescript
import { activate, validate, hasEntitlement } from 'tauri-plugin-keylight-api';

await activate('USER-LICENSE-KEY');
const ok = await validate();
if (await hasEntitlement('pro')) {
  // unlock pro features
}
```

## Foreground revalidation

Wire `activeRevalidate()` to your window's focus event so a dashboard revoke lands within minutes
of the user next touching the app, instead of waiting for the normal refresh cadence or a
relaunch. It's debounced to 60s on the Rust side, so a hammered focus event is safe to call
directly.

**From the frontend** (idiomatic Tauri v2 pattern, using `@tauri-apps/api/window`):

```typescript
import { getCurrentWindow } from '@tauri-apps/api/window';
import { activeRevalidate } from 'tauri-plugin-keylight-api';

getCurrentWindow().onFocusChanged(({ payload: focused }) => {
  if (focused) {
    activeRevalidate();
  }
});
```

**From Rust**, the equivalent is handling `WindowEvent::Focused(true)` in `setup()` or via
`on_window_event` on the app builder — call `Keylight::active_revalidate()` directly on the
`Arc<Keylight>` you already manage as plugin state, rather than round-tripping through the
`active_revalidate` command.

## Post-upgrade refresh

After a customer completes an upgrade (e.g. returning from a hosted billing portal), call
`refreshAfterUpgrade()` to poll-revalidate briefly so the new entitlements unlock in the running
app without waiting for the normal refresh cadence:

```typescript
import { refreshAfterUpgrade } from 'tauri-plugin-keylight-api';

const changed = await refreshAfterUpgrade(); // polls up to 30s, every 2s by default
if (changed) {
  // re-check hasEntitlement(...) / refresh UI state
}
```

This call blocks on the Rust side for up to the timeout, so await it rather than calling it from a
hot path (e.g. trigger it once when the app regains focus after an upgrade flow, not on every
render).

## License

MIT
