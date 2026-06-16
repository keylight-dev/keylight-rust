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

`keylight:default` allows `activate`, `validate`, and `has_entitlement`. You can also grant the
per-command permissions individually (`keylight:allow-activate`, `keylight:allow-validate`,
`keylight:allow-has-entitlement`).

## Use from the frontend

```typescript
import { activate, validate, hasEntitlement } from 'tauri-plugin-keylight-api';

await activate('USER-LICENSE-KEY');
const ok = await validate();
if (await hasEntitlement('pro')) {
  // unlock pro features
}
```

## License

MIT
