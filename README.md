# keylight-rust

Open-source [Keylight](https://keylight.dev) licensing SDK for Rust — the
`keylight` library plus a `keylight` CLI (`keylight-cli`). Activate license
keys, verify signed leases offline, and gate features on entitlements.

Verified against the Keylight SP-0 conformance vectors (the crate passes
`tests/conformance.rs`).

## Install

```toml
[dependencies]
keylight = "0.1"
```

Not yet published to crates.io. Until then, use a git dependency:

```toml
[dependencies]
keylight = { git = "https://github.com/keylight-dev/keylight-rust" }
```

## Quickstart (library)

```rust
use keylight::{Keylight, KeylightConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build config. Fetch the tenant's trusted Ed25519 keyset so leases can be
    // verified offline. (Alternatively pin keys with .trusted_key(kid, pub_b64).)
    let mut cfg = KeylightConfig::builder("keylight-notes-demo", "notes")
        .key_prefix("NOTES")
        .build();
    if let Some((_, keys)) = keylight::keyset::fetch_keyset(
        &keylight::http::ureq_transport::UreqTransport::default(),
        &cfg.base_url, &cfg.tenant_id) {
        cfg.trusted_keys.extend(keys);
    }
    let kl = Keylight::new(cfg)?;

    // Activate a license key (online). The returned lease is Ed25519-verified
    // before anything is persisted.
    let res = kl.activate("NOTES-PRO0-0000-0001")?;
    println!("activated: {}", res.activated);

    // Gate features on entitlements (works offline from the cached lease).
    if kl.has_entitlement("pro") {
        println!("Pro features unlocked");
    }
    Ok(())
}
```

This example runs against the public Keylight Notes demo tenant
(`keylight-notes-demo`, product `notes`) on the default host
`https://api.keylight.dev`.

## Offline model

The signed `v3` lease is the offline artifact: once activated, it's persisted
locally and every entitlement check reads from it without a network call. Call
`refresh_if_needed()` (or `check_on_launch()`) on launch and on relevant app
events to renew the lease when it's near expiry — there are no background
threads. Leases are verified with Ed25519 against the trusted keyset, allowing
300s of clock skew.

## CLI usage

The `keylight` binary wraps the same library:

```sh
keylight --tenant keylight-notes-demo --product notes --fetch-keys activate NOTES-PRO0-0000-0001
keylight --tenant keylight-notes-demo --product notes --fetch-keys status
keylight --tenant keylight-notes-demo --product notes deactivate
```

`--fetch-keys` pulls the tenant's trusted keyset from the server. Flags also
read from environment variables: `KEYLIGHT_TENANT`, `KEYLIGHT_PRODUCT`,
`KEYLIGHT_SDK_KEY`, and `KEYLIGHT_BASE_URL` (defaults to
`https://api.keylight.dev`).

## Crates

| Crate | Description |
| --- | --- |
| `keylight` | The licensing library. |
| `keylight-cli` | The `keylight` command-line binary. |
| `demo-app` | Keylight Notes — a runnable example app. |
| `tauri-plugin-keylight` | Tauri plugin wrapping the SDK. |

## License

MIT.
