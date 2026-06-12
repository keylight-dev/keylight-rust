# Contributing

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The security-critical lease verifier is gated by Keylight's frozen **cross-SDK conformance vectors**
(`keylight/tests/conformance.rs`) — the canonical source for the whole SDK family. Don't loosen or
skip them; a failing vector means the verifier has diverged from the other SDKs.

## Releasing (maintainers)

Versions are published via tag-triggered CI (`.github/workflows/release.yml`) using **tokenless
OIDC trusted publishing** — no tokens in the repo.

```bash
# 1. Bump the workspace version in Cargo.toml, update CHANGELOG if present.
# 2. Verify locally:
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo publish -p keylight --dry-run

# 3. Commit, then tag — the tag fires the release workflow:
git commit -am "Release vX.Y.Z"
git push origin main
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push origin vX.Y.Z
```

The workflow publishes the `keylight` crate to crates.io and attaches per-platform `keylight` CLI
binaries (macOS, Linux, Windows) to the GitHub Release.
