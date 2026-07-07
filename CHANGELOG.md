# Changelog

All notable changes to the Keylight Rust SDK are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.3.1]: https://github.com/keylight-dev/keylight-rust/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/keylight-dev/keylight-rust/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/keylight-dev/keylight-rust/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/keylight-dev/keylight-rust/releases/tag/v0.1.3
