# Keylight Rust SDK

[![Crates.io](https://img.shields.io/crates/v/keylight.svg)](https://crates.io/crates/keylight)
[![Documentation](https://docs.rs/keylight/badge.svg)](https://docs.rs/keylight)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Edition](https://img.shields.io/badge/rust-2024-orange.svg)](https://www.rust-lang.org)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-blue.svg)](https://www.rust-lang.org)
[![Conformance](https://img.shields.io/badge/conformance-cross--SDK%20vectors-success.svg)](#conformance)

Open-source Rust SDK and Tauri plugin for [Keylight](https://keylight.dev) — license your Rust
apps, CLIs, daemons, and Tauri desktop apps with online activation and offline Ed25519 license
verification.

> **In one line:** a software-licensing SDK for Rust — license-key activation and validation,
> entitlement/feature gating, trials and free tiers, and tamper-resistant **offline license
> verification** (signed `v3` lease, Ed25519 + clock-skew tolerance) for CLIs, daemons, and
> desktop apps. Synchronous and runtime-free — no `async`/Tokio required.

## Why Keylight

Licensing shouldn't mean bolting a heavyweight, phone-home-or-die SDK onto your app.

- **Works offline.** The license is a signed lease your app verifies locally with Ed25519 — no
  network round-trip to gate a feature, no lockout when the machine is offline.
- **Tamper-resistant by design.** Entitlements live *inside* the signature; a forged or hand-edited
  lease can't pass verification without the tenant's private key.
- **Synchronous & runtime-free.** No `async`/Tokio, no background threads — you call `activate` /
  `refresh_if_needed` on launch and on events, and decide exactly when to check in. Ideal for CLIs
  and daemons.
- **One SDK family.** Verifies licenses identically to the Swift and JavaScript SDKs, proven by
  shared conformance vectors.

## Table of Contents

- [Why Keylight](#why-keylight)
- [Features](#features)
- [Packages](#packages)
- [Quick Start](#quick-start)
  - [Pure Rust](#pure-rust)
  - [Tauri Apps](#tauri-apps)
- [License Lifecycle](#license-lifecycle)
- [License States](#license-states)
- [Entitlements](#entitlements)
- [Offline Validation](#offline-validation)
- [Refresh, Trials & Free Tier](#refresh-trials--free-tier)
- [Lifecycle Events](#lifecycle-events)
- [Configuration Reference](#configuration-reference)
  - [Signed product settings](#signed-product-settings)
  - [Post-upgrade refresh](#post-upgrade-refresh)
- [CLI & Demo](#cli--demo)
- [Conformance](#conformance)
- [Documentation](#documentation)
- [Other SDKs](#other-sdks)
- [License](#license)

## Features

- **License Lifecycle** — Activate, validate, and deactivate license keys with a small, explicit API.
- **Offline Verification** — The single offline artifact is a signed `v3` **lease**, verified with
  **Ed25519** and a 300-second clock-skew tolerance. An optional `max_offline_days` grace caps how
  long a device may run without checking in.
- **Synchronous & runtime-free** — Blocking HTTP (`ureq`); no `async`/Tokio, no background threads.
  You call `refresh_if_needed()` / `check_on_launch()` on launch and on app events. Ideal for CLIs
  and daemons.
- **Entitlements** — Feature gating from the cached lease: `has_entitlement("pro")`.
- **Trials & Free Tier** — Built-in local trial timer, free-tier mode, and an anonymous "keyless"
  usage beacon.
- **Lifecycle Events** — Optional callback fires `Renewed` / `Cancelled` / `Expired` / `Restored`
  as the resolved state changes.
- **Clock-Manipulation Detection** — Flags backward/forward system-clock tampering.
- **Device Telemetry** — Auto-attaches `sdk_version` and `platform`. To also report your app
  version, you **must** set it with `.app_version(env!("CARGO_PKG_VERSION"))` on the builder — the
  SDK can't infer it, so it's omitted (and the dashboard shows a blank version) until you do.
- **Network Resilience** — Automatic retry with exponential backoff + jitter; honors `Retry-After`.
- **Secure by Default** — TLS via **rustls** (no OpenSSL), **ChaCha20-Poly1305** device-bound
  encrypted on-disk storage, and **no `unsafe`** in the SDK crate.
- **Pluggable** — Swap the storage backend (`LicenseStore`) or HTTP transport (`Transport`) via
  traits for tests or custom platforms.

## Packages

This workspace contains:

| Crate | Description | Distribution |
|-------|-------------|--------------|
| [`keylight`](./keylight) | Core Rust SDK for any Rust application | [![crates.io](https://img.shields.io/crates/v/keylight.svg)](https://crates.io/crates/keylight) [![docs](https://docs.rs/keylight/badge.svg)](https://docs.rs/keylight) |
| [`keylight-sdk-demo`](./keylight-sdk-demo) | Reference CLI / template for white-labeled `yourapp activate` commands (also a dev/ops & CI utility). Ships as `keylight-demo` — **not** the [Keylight management CLI](https://github.com/keylight-dev/keylight-cli), which is `keylight`. | Prebuilt binaries on [GitHub Releases](https://github.com/keylight-dev/keylight-rust/releases) |
| [`tauri-plugin-keylight`](./tauri-plugin-keylight) | Tauri v2 plugin (Rust side) with capability permissions | [![crates.io](https://img.shields.io/crates/v/tauri-plugin-keylight.svg)](https://crates.io/crates/tauri-plugin-keylight) |
| [`tauri-plugin-keylight-api`](./tauri-plugin-keylight) | Tauri v2 plugin JS/TS bindings (ESM/CJS + `.d.ts`) | [![npm](https://img.shields.io/npm/v/tauri-plugin-keylight-api.svg)](https://www.npmjs.com/package/tauri-plugin-keylight-api) |
| [`keylight-notes-demo`](./demo-app) | "Keylight Notes" example app | Example (not published) |

## Quick Start

### Pure Rust

```bash
cargo add keylight
```

```rust
use keylight::{Keylight, KeylightConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build a config. Fetch the tenant's trusted Ed25519 keyset so leases can be
    // verified offline. (You can also pin keys explicitly with `.trusted_key(kid, pub_b64)`.)
    let mut cfg = KeylightConfig::builder("your-tenant", "your-product", "sdk_live_…")
        .key_prefix("PROD")                       // optional client-side key-format check
        .max_offline_days(7)                      // optional offline grace window
        .app_version(env!("CARGO_PKG_VERSION"))   // report app version (omitted if unset)
        .build();
    if let Some((_, keys)) = keylight::keyset::fetch_keyset(
        &keylight::http::ureq_transport::UreqTransport::default(),
        &cfg.base_url,
        &cfg.tenant_id,
    ) {
        cfg.trusted_keys.extend(keys);
    }

    let kl = Keylight::new(cfg)?;

    // Activate a license key (online). The returned lease is Ed25519-verified
    // *before* anything is persisted.
    let res = kl.activate("USER-LICENSE-KEY")?;
    println!("activated: {}", res.activated);

    // Gate features on entitlements — works offline from the cached lease.
    if kl.has_entitlement("pro") {
        println!("Pro features unlocked");
    }

    // Release the seat when uninstalling / switching devices.
    kl.deactivate()?;
    Ok(())
}
```

> Note the **synchronous** API — there is no `.await` and no async runtime to set up.

### Tauri Apps

Add the Rust-side Tauri v2 plugin and the JS bindings:

```bash
# Rust side
cargo add tauri-plugin-keylight
# JavaScript side
npm add tauri-plugin-keylight-api
```

Register it with a prebuilt `KeylightConfig` (your app supplies tenant/product/keys):

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

Grant the plugin's default permission set in your capability file:

```json
// src-tauri/capabilities/default.json
{
  "permissions": ["keylight:default"]
}
```

`keylight:default` allows `activate`, `validate`, and `has_entitlement` (per-command permissions
`keylight:allow-activate` etc. are also generated).

Use the typed JS/TS bindings ([`tauri-plugin-keylight-api`](https://www.npmjs.com/package/tauri-plugin-keylight-api),
ESM/CJS + `.d.ts`) from your frontend:

```typescript
import { activate, validate, hasEntitlement } from 'tauri-plugin-keylight-api';

await activate('USER-LICENSE-KEY');
const ok = await validate();
if (await hasEntitlement('pro')) {
  // unlock pro features
}
```

## License Lifecycle

```
┌─────────────┐     ┌─────────────┐     ┌──────────────┐
│   activate  │────▶│   validate  │────▶│  deactivate  │
└─────────────┘     └─────────────┘     └──────────────┘
                          ▲
                          │ on launch / on events (no background threads)
                    ┌───────────────────┐
                    │ refresh_if_needed │
                    └───────────────────┘
```

| Method | Description |
|--------|-------------|
| `activate(key) -> ActivationResult` | Activates a key on this device. Verifies the returned lease before persisting; returns `instance_id`, the lease, and expiry. |
| `validate() -> ValidationResult` | Re-checks the stored license online. Decodes hard-expiry (`422`) responses and preserves fallback/expired leases so state can resolve. |
| `deactivate()` | Releases the seat and clears local license state. Call on uninstall or device switch. |
| `refresh_if_needed() -> Option<ValidationResult>` | Validates only if due (debounce 5 min, stale 6 h, or within 24 h of expiry). Safe to call often. |
| `check_on_launch()` | Convenience: refresh if a license is stored, else no-op. |
| `active_revalidate() -> Option<ValidationResult>` | Forces a validate on active use (foreground / focus / popover), debounced 60 s in memory. Catches a revoke mid-session; a network blip never downgrades. |

## License States

`state()` resolves a single high-level status from the cached lease, trial, and free-tier config
(no network):

| State | Meaning |
|-------|---------|
| `Licensed` | Current, signature-valid `active` lease. |
| `Limited` | Signature-valid `fallback` lease (grace mode). |
| `Trial { days_left }` | No license, but a local trial is active. |
| `FreeTier` | No license, free tier enabled. |
| `Expired` | Lease expired, or a license was stored but is no longer current. |
| `Invalid` | No license, no trial, no free tier. |

## Entitlements

Entitlements are feature keys carried inside the signed lease and checked offline:

```rust
if kl.has_entitlement("cloud-sync") {
    enable_cloud_sync();
}
```

`has_entitlement` returns `true` only when the cached lease is signature-valid, unexpired, and not
`expired`-status — so offline feature gating never disagrees with the resolved `Expired` state.

## Offline Validation

The offline artifact is a signed **`v3` lease** issued by the Keylight API. The SDK reconstructs
the exact signed payload (entitlements sorted, pipe-delimited) and verifies it with **Ed25519**
against the tenant's trusted keyset, applying a **300-second clock-skew** tolerance:

```rust
use keylight::KeylightConfig;

let cfg = KeylightConfig::builder("your-tenant", "your-product", "sdk_live_…")
    // Pin trusted keys explicitly instead of fetching them:
    .trusted_key("k1", "<raw ed25519 public key, base64>")
    .max_offline_days(7) // None = run offline as long as the lease itself is current
    .build();
```

- The trusted keyset can be fetched once from `GET /{tenant}/.well-known/keylight-keys`
  (`keylight::keyset::fetch_keyset`) or pinned at build time.
- `cached_lease()` returns the lease only when it is `kid`-known, signature-valid, unexpired, and
  (if set) within `max_offline_days` of the last online validation.
- **Key rotation:** every lease carries a `kid`, and a lease whose `kid` is absent from the keyset is
  rejected (`VerifyResult { kid_known: false }`). `.trusted_key()` is chainable (and
  `trusted_keys.extend(...)` bulk-adds), so when the tenant rotates its signing key you bump the `kid`
  (`k1` → `k2`) and register **both** the outgoing and incoming keys for a deprecation window —
  leases signed by either still verify, so unexpired leases don't flip to invalid mid-session.
  If you `fetch_keyset` instead of pinning, the well-known endpoint already returns every active
  `kid`, so rotation is transparent. (See your Keylight tenant's `security.md` → *Ed25519 key rotation*.)
- Encrypted lease/key material is stored device-bound with **ChaCha20-Poly1305** (key derived from a
  per-device identity via BLAKE3) — copying the files to another machine won't decrypt them.

## Refresh, Trials & Free Tier

There are **no background threads**. The host drives refresh on launch and on meaningful events:

```rust
kl.check_on_launch()?;       // validate if due, on startup
kl.refresh_if_needed()?;     // call again on window-focus / purchase / resume
kl.active_revalidate();      // force a check when the user brings the app forward
```

`refresh_if_needed()` respects a staleness policy (5 min debounce, 6 h stale), so with a long-lived
lease a revoke can sit unnoticed until the next launch. `active_revalidate()` is the prompt
counterpart: it always calls the server (debounced 60 s, in memory only), downgrades immediately on a
definitive rejection, and leaves the session untouched on a transient failure.

Trials and free tier are local and offline-first:

```rust
kl.start_trial()?;                       // begins the trial clock once
match kl.check_trial() {                 // NotStarted | Active { days_left } | Expired
    keylight::TrialStatus::Active { days_left } => println!("{days_left} days left"),
    _ => {}
}

// Anonymous, debounced usage beacon for trial / free-tier / expired devices.
// It carries `sdk_version` and `platform` automatically; to also see your app
// version on these devices in the dashboard, set `.app_version(…)` on the
// builder (see the client setup above) — it is omitted otherwise.
kl.report_keyless_state(keylight::KeylessState::Trial);

// Tamper check and a pre-filled hosted upgrade link:
// `state()` already forces `Invalid` if the clock was rolled back past tolerance
// (the offline vector for reviving an expired lease). `is_clock_manipulated()`
// additionally surfaces large forward jumps if you want to react to them too.
let tampered = kl.is_clock_manipulated();
if let Some(url) = kl.upgrade_url() { println!("Upgrade: {url}"); }
```

## Lifecycle Events

Register a handler to react when the resolved state crosses a transition:

```rust
use keylight::{Keylight, LicenseLifecycleEvent};

let kl = Keylight::new(cfg)?
    .with_event_handler(|event| match event {
        LicenseLifecycleEvent::Renewed   => println!("license renewed"),
        LicenseLifecycleEvent::Cancelled => println!("dropped to limited/expired"),
        LicenseLifecycleEvent::Expired   => println!("license expired"),
        LicenseLifecycleEvent::Restored  => println!("license restored"),
    });
```

| Event | Fires when |
|-------|-----------|
| `Renewed` | Stayed `Licensed` and the expiry moved later. |
| `Cancelled` | `Licensed` → `Limited` or `Expired`. |
| `Expired` | Any state → `Expired`. |
| `Restored` | `Expired`/`Limited`/`Invalid` → `Licensed`. |

Events are evaluated during `validate()` and re-derive the previous state from the persisted lease,
so a transition won't re-fire across restarts.

## Configuration Reference

Built with `KeylightConfig::builder(tenant_id, product_id, sdk_key)`:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `tenant_id` | `String` | — | Your Keylight tenant (required). |
| `product_id` | `String` | — | Your product (required). |
| `sdk_key` | `String` | — | Tenant SDK key (required), sent as `X-Keylight-SDK-Key` on every call. |
| `trusted_keys` | `map<kid, pub>` | empty | Trusted Ed25519 public keys for offline verification (`.trusted_key()` or `fetch_keyset`). |
| `max_offline_days` | `Option<u32>` | `Some(15)` | Offline grace window since last online validation. `None` = until the lease itself expires. |
| `require_signed_config` | `bool` | `false` | Require server-owned product settings to carry a valid Ed25519 signature before they are cached. See [Signed product settings](#signed-product-settings). |
| `trial_duration_days` | `u32` | `0` | Local trial-length *seed* (days). `0` = no trial unless the dashboard says otherwise. |
| `free_tier_enabled` | `bool` | `false` | Resolve to `FreeTier` when there's no license/trial. |
| `app_version` | `Option<String>` | `None` | Reported in telemetry. |
| `base_url` | `String` | `https://api.keylight.dev` | API base URL. |
| `key_prefix` | `Option<String>` | `None` | Client-side key-format check (e.g. `"PROD"`). |

### Signed product settings

The trial length and free-tier flag are **server-owned**: the Keylight worker delivers them on
`/config`, on `validate`, and on the keyless beacon, and the SDK caches whatever it last heard.
`require_signed_config` makes the SDK verify an Ed25519 signature over those settings before caching
them; settings that do not verify are dropped and the compiled-in seed is kept.

```rust
let cfg = KeylightConfig::builder("your-tenant", "your-product", "sdk_live_…")
    .trusted_key("k1", "<raw ed25519 public key, base64>")
    .require_signed_config(true)
    .build();
```

- **Off by default — leave it off unless your product is signed.** The worker signs a product's
  settings only once that product has a **trial length configured in the dashboard**; every other
  product is served unsigned, and enabling this against one of those rejects every legitimate
  response and pins the install to its seed.
- **Keys are compiled in via `trusted_keys`, never fetched at runtime.** A keyset fetched over the
  same connection that serves the settings would let anyone able to forge one forge the other, so
  the SDK deliberately does not do that.
- **Rotation freezes, it does not break.** A build that only knows the old `kid` stops accepting new
  settings and stays on its last cached values until it ships with the new key. Register both keys
  for a deprecation window, exactly as for lease verification.
- `verify_config(&fields, tenant_id, product_id, &trusted_keys, now_seconds, skew_seconds) -> bool`
  is the check itself, exported for hosts that want to verify a `ProductConfigFields` payload
  themselves. `tenant_id`/`product_id` come from *your* config, not the payload, so a config signed
  for another product fails rather than validating against its own claim. Freshness (`issued_at` /
  `expires_at`, with `skew_seconds` tolerance, `keylight::SKEW_SECONDS` = 300) applies to the wire
  only — an already-cached config stays usable past its window.

### Post-upgrade refresh

`refresh_after_upgrade(timeout: Duration, poll_interval: Duration) -> bool` briefly poll-revalidates
after a customer completes an upgrade, so new entitlements (or a mid-flight rejection) show up in the
running app without waiting for the normal refresh cadence. It covers payment-webhook lag: checkout
can finish in the browser slightly before the provider's webhook reaches Keylight.

```rust
use std::time::Duration;

let kl = kl.clone(); // Arc<Keylight>
std::thread::spawn(move || {
    if kl.refresh_after_upgrade(Duration::from_secs(30), Duration::from_secs(2)) {
        // entitlement set or state changed — refresh the UI
    }
});
```

- Re-validates every `poll_interval` (clamped to a **100 ms floor**) until the entitlement set or
  the resolved state changes, or `timeout` elapses. Returns `true` on a change, `false` on timeout
  or when no license is stored (no network call in that case).
- **Blocking.** It sleeps between attempts and can take up to `timeout` to return (each sleep is
  capped to the time remaining, so it never overshoots). Run it on a background thread, never on a
  UI/main thread. The Tauri plugin's `refreshAfterUpgrade()` already does this for you.
- A seat-only upgrade with no entitlement/state change is invisible to it and runs to `timeout`; the
  normal refresh cadence still picks that up.

## CLI & Demo

### `keylight-sdk-demo` — a reference to build on, not a product to rebrand

`keylight-sdk-demo` is a **reference implementation**: a thin [clap](https://docs.rs/clap) wrapper
around the SDK (see [`keylight-sdk-demo/src/main.rs`](./keylight-sdk-demo/src/main.rs)). It installs as
`keylight-demo`, deliberately not `keylight` — that name belongs to the
[Keylight management CLI](https://github.com/keylight-dev/keylight-cli). Its main purpose is
to be the worked example for **adding white-labeled licensing commands to your own CLI**.

You don't ship this binary renamed — you embed the `keylight` *library* in **your** tool, bake in
your tenant/product, and expose your own branded subcommand. The end user then just runs
`yourapp activate <KEY>` (no `--tenant`/`--product` to pass):

```rust
// In your CLI `mole`: `mole activate <KEY>`
Cmd::Activate { key } => {
    let kl = Keylight::new(KeylightConfig::builder("mole-co", "mole", "sdk_live_…").build())?;
    let unlocked = kl.activate(&key)?.activated;
    println!("{}", if unlocked { "Mole Pro unlocked 🎉" } else { "Invalid key" });
}
// gate features elsewhere:  if kl.has_entitlement("pro") { /* ... */ }
```

You can also run the **generic** binary as-is — useful for **local development, testing a tenant,
or exit-code gating in scripts/CI** (it is not a customer-facing tool):

```bash
cargo install --git https://github.com/keylight-dev/keylight-rust keylight-sdk-demo

keylight --tenant your-tenant --product your-product --fetch-keys activate USER-LICENSE-KEY
keylight --tenant your-tenant --product your-product validate || echo "license invalid"
```

### `keylight-notes-demo`

The demo app shows entitlement gating end-to-end (free = 3 notes; the `pro`
entitlement unlocks unlimited notes + export) against the live public demo tenant:

```bash
cargo run -p keylight-notes-demo -- add "first note"
cargo run -p keylight-notes-demo -- activate NOTES-PRO0-0000-0001
cargo run -p keylight-notes-demo -- export /tmp/notes.txt   # pro-only
```

## Conformance

The security-critical lease verifier is gated by Keylight's frozen **cross-SDK conformance vectors**
(`keylight/tests/conformance.rs`). The Rust verifier must agree with every vector on
`{ kid_known, signature_valid, expired }`, which keeps offline verification behavior identical
across the Keylight SDK family (Swift, Rust, …).

```bash
cargo test -p keylight --test conformance
```

## Documentation

- **API docs (docs.rs):** [docs.rs/keylight](https://docs.rs/keylight)
- **Platform docs:** [docs.keylight.dev](https://docs.keylight.dev)
- **Website:** [keylight.dev](https://keylight.dev)
- **API host:** `https://api.keylight.dev`

## Other SDKs

| Platform | Status | Repository |
|----------|--------|------------|
| Swift (macOS/iOS) | Available | [keylight-swift](https://github.com/keylight-dev/keylight-swift) |
| Rust (this repo) | Available | [keylight-rust](https://github.com/keylight-dev/keylight-rust) |
| JavaScript/TypeScript | Available | [keylight-js](https://github.com/keylight-dev/keylight-js) |
| C# · C++ | Planned | unified by the same cross-SDK conformance vectors |

## About Keylight

Keylight is the licensing layer for desktop apps. You keep your own Stripe account,
your own pricing, and your own customers — Keylight issues the licenses and tells your
app who is allowed to run it.

- **License keys** issued automatically when a payment completes
- **Device activations** with limits you set, and self-serve deactivation
- **Offline validation** — signed Ed25519 leases your app verifies locally
- **Feature entitlements** signed into the lease, so tiers work offline too

[keylight.dev](https://keylight.dev) · [Documentation](https://docs.keylight.dev) · [Pricing](https://keylight.dev/pricing)

### Further reading

- [Offline License Validation in Rust](https://keylight.dev/blog/offline-license-validation-rust)
- [How to Add License Keys to a Tauri App](https://keylight.dev/blog/add-license-keys-tauri-app)
- [What Is Inside a Keylight Lease: The Ed25519 Format Explained](https://keylight.dev/blog/ed25519-lease-format-explained)

## License

MIT License. See [LICENSE](LICENSE) for details.

---

<sub>Keylight Rust SDK — software licensing for Rust: license-key activation & validation, offline
Ed25519 lease verification, entitlement/feature gating, trials and free tiers, device-bound
encrypted storage, and a Tauri v2 plugin for CLIs, daemons, and desktop apps.</sub>
