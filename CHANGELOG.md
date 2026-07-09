# Changelog

All notable changes to the Keylight Rust SDK are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.3.2]: https://github.com/keylight-dev/keylight-rust/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/keylight-dev/keylight-rust/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/keylight-dev/keylight-rust/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/keylight-dev/keylight-rust/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/keylight-dev/keylight-rust/releases/tag/v0.1.3
