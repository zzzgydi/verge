# Verge GPUI application

`verge-gpui` keeps a separate Cargo workspace during the migration. The legacy Tauri beta and
the current GPUI stack require incompatible `raw-window-handle` and `web-sys` lock versions, so
sharing one lockfile would force unrelated dependency upgrades. Run
`scripts/check-rust-workspaces.sh` from the repository root to validate both Rust workspaces.

This is the formal GPUI window target. It consumes `verge-ui` state and emits only typed
domain requests; views do not call Mihomo or macOS APIs directly.

The app owns the Mihomo lifecycle and provides Home, Proxies, Profiles, Connections, Logs and
Settings pages plus a native dynamic tray.

```bash
cargo +1.97.1 check --manifest-path apps/verge-gpui/Cargo.toml
cargo +1.97.1 run --manifest-path apps/verge-gpui/Cargo.toml
```

Build an Apple Silicon `.app` with the pinned sidecar:

```bash
VERGE_MIHOMO_BIN=/absolute/path/to/mihomo \
  apps/verge-gpui/scripts/build-macos-app.sh
```

The result is `dist/Verge.app`. The script verifies the sidecar SHA-256 and uses an ad-hoc
signature by default. Set `VERGE_CODESIGN_IDENTITY` to a Developer ID Application identity for a
release candidate.
