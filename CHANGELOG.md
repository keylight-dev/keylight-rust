# Changelog

All notable changes to the Keylight Rust SDK are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.6] - 2026-08-07

### Added

- **New optional device fields on activate / validate / keyless.** The SDK now
  reports the device's OS version (dotted-numeric, e.g. `15.5` on macOS, the
  kernel release on Linux) and CPU architecture (`arm64` / `x86_64`) alongside
  the existing telemetry fields. Both are collected with platform APIs already
  in use — no new dependencies — and omitted when they can't be read cleanly.
  No API change and nothing to do in your code.
- **Coarse hardware shape on the same requests.** `cpu_cores` and `memory` are
  reported as ranges (`5-8`, `16-32GB`), never as exact values — the precise
  core count and RAM size never leave the machine. Read via
  `std::thread::available_parallelism` and a per-OS memory query, with no new
  dependencies, and omitted when a platform can't be read cleanly.

## [0.3.5] - 2026-08-01

### Added

- **The SDK now identifies itself on the wire.** `activate` and `validate` send
  `sdk: "rust"` alongside the existing `platform` field. `platform` reports the
  operating system and nothing more, which made it identical across the Rust,
  C++ and C# SDKs — it could not say which SDK a device was running. No API
  change and nothing to do in your code.

## [0.3.4] - 2026-07-29

### Added

- **`active_revalidate()` — prompt revocation enforcement on active use.** A new
  primitive (Swift parity with `activeRevalidate()`) that forces a server
  validate when the user brings the app forward (foreground / window focus /
  popover open), bypassing the staleness gates `refresh_if_needed()` applies.
  Debounced to 60 s **in memory**, so the window never survives a process
  restart. A definitive rejection (e.g. a dashboard revoke's HTTP 422)
  downgrades the session immediately; a transient/network failure returns
  `None` and leaves state exactly as it was — a blip never downgrades a live
  session. Previously a revoke could not land mid-session until the lease went
  stale (6 h) or the app relaunched.

### Fixed

- **Telemetry values are clamped to the server's limits.** `app_version` (which
  the host app supplies) is truncated to 64, `platform` to 32, before being sent
  on activate / validate / keyless. An over-long value was rejected by the API
  with a 400 for the *whole* request — so an unusually long app version string
  could fail activation outright, not just lose the field. Parity with the JS
  SDK.

  The limit is measured in UTF-16 code units, matching what the API actually
  enforces. Counting `char`s instead would disagree for text outside the BMP —
  64 emoji are 64 `char`s but 128 code units — so a value the SDK considered
  clamped could still have been rejected.

## [0.3.3] - 2026-07-17

### Added

- **`machine_hash` on activate and validate.** The same cross-SDK device hash
  the keyless beacon sends is now attached to `activate` and `validate`
  requests, so the dashboard counts a device that converts from keyless to
  licensed (or keeps validating) as **one** daily-active device instead of two.
  Omitted, as before, when no stable hardware id is available.
- **Tauri plugin: three new commands** — `checkOnLaunch()` (server validation
  with no staleness gate, for app launch so a dashboard revoke takes effect
  immediately), `refreshIfNeeded()` (re-validate only when the SDK's
  debounce/staleness policy says so; returns `null` when skipped), and
  `reportKeylessState(state)` (the anonymous keyless beacon, debounced 24h in
  the SDK). Each has its own permission and all three are included in the
  plugin's `default` permission set.
- **Tauri plugin: optional built-in heartbeat scheduler.**
  `init_with_heartbeat(config, HeartbeatOptions)` spawns a background thread
  that periodically calls `refresh_if_needed` and, in keyless states, sends the
  keyless beacon (default every 6h, floor 60s). Off by default — `init()` is
  unchanged.

### Fixed

- **Hardware id is cached for stability.** The hardware id is persisted on
  every successful OS read and reused on a transient read failure, so the
  derived `machine_hash` stays stable across beacons instead of silently
  disappearing (which would have created a second device server-side). There is
  still no random fallback: if no id has ever been read, the field is omitted.
- **Keyless beacon now uses the shared retry/backoff loop.** A transient
  network failure or 5xx no longer silently drops the beacon; the 24h debounce
  state is persisted only on a confirmed HTTP 200, so a failed send is retried
  on the next opportunity instead of being suppressed for a day.
- **`deactivate` now carries telemetry** (`app_version`/`sdk_version`/
  `platform`) like every other route.
- **macOS `IOPlatformUUID` parsing** trims whitespace and rejects empty values
  instead of deriving a hash from a blank id.

## [0.3.2] - 2026-07-09

### Added

- **Privacy-safe machine identity on keyless beacons.** The keyless/free-tier
  heartbeat now sends a one-way `machine_hash` derived from a stable hardware
  identifier (`IOPlatformUUID` on macOS, `/etc/machine-id` on Linux,
  `MachineGuid` on Windows), namespaced to your tenant and product. It lets the
  dashboard count one device per physical machine instead of per install — a
  reinstall updates the same free-tier row rather than creating a duplicate —
  while the raw hardware ID never leaves the device (only the SHA-256 hash is
  sent). Omitted automatically when no stable hardware ID is available, so
  headless/unsupported platforms fall back to the existing per-install id.
  Byte-for-byte identical to the Swift and JS SDKs for the same inputs. Inject a
  custom identity for tests with `Keylight::with_device(...)`.

### Fixed

- **Revocation now enforced; offline use bounded to 15 days.** Launch always performs
  a server `validate` (no staleness gating), so a dashboard revoke or expiry lands on
  the next launch instead of lagging the refresh cadence. A definitive server rejection
  with no lease clears the stale cached lease instead of leaving a "still-active" lease
  in place.
- **Offline cap is fail-closed on a missing online anchor.** `state()` skipped the
  `max_offline_days` check when no `last_validated_online` timestamp was stored, so a
  signature-valid cached lease still resolved to `Licensed` — letting anyone who deletes
  the anchor reset the offline clock indefinitely. A missing *or* stale anchor now drops
  the lease (parity with `cached_lease()` and the Swift SDK's `isWithinOfflineGrace`).
  `max_offline_days = None` still disables the cap; trials and free-tier are unaffected.

## [0.3.1] - 2026-07-07

### Documentation

- Document that `KeylightConfig::builder(...).app_version(...)` must be called
  explicitly to report an app version. Unlike `sdk_version` and `platform`
  (attached automatically), the app version is unknown to the SDK and is omitted
  from every request — including the keyless/free-tier beacon — unless set, which
  left it blank in the dashboard. Added a doc-comment with an example, corrected
  the README telemetry bullet, and added `.app_version(env!("CARGO_PKG_VERSION"))`
  to the setup examples.

## [0.3.0] - 2026-06-25

- Migrate the workspace to Rust edition 2024 and declare MSRV 1.85.
- Align the `tauri-plugin-keylight-api` npm package version to 0.3.0.

## [0.2.0] - 2026-06-16

- Require the tenant SDK key on the client (`X-Keylight-SDK-Key`).
- Add the backward-clock-rollback guard to offline state resolution.

## [0.1.3] - 2026-06-11

- Earlier 0.1.x releases: initial Rust SDK with online activation, offline
  Ed25519 lease verification, trials, free-tier/keyless beacon, entitlements,
  and the first-party Tauri v2 plugin.

[0.3.3]: https://github.com/keylight-dev/keylight-rust/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/keylight-dev/keylight-rust/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/keylight-dev/keylight-rust/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/keylight-dev/keylight-rust/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/keylight-dev/keylight-rust/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/keylight-dev/keylight-rust/releases/tag/v0.1.3
