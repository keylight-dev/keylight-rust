# Changelog

All notable changes to the Keylight Rust SDK are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
