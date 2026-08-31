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
release candidate. The script also stamps `CFBundleShortVersionString` / `CFBundleVersion` from
`Cargo.toml`, which is what the in-app updater reports as the current version.

## Application self-update

The Settings page can check `github.com/zzzgydi/verge` releases and update the installed
`Verge.app` in place. Release format contract (see `crates/verge-application/src/app_update.rs`
for the full rules):

- Release tag is a semantic version with optional `v` prefix; drafts and pre-releases are skipped.
- The macOS arm64 asset is `Verge-macos-arm64.zip`, containing a single top-level `Verge.app`.
- The sidecar asset `Verge-macos-arm64.zip.sha256` carries the `shasum -a 256` digest line.

An update verifies the zip digest, the bundle structure and its `codesign --verify --deep
--strict` signature, and requires the same signing identity as the running instance before
swapping. The previous bundle is kept under the data directory (`updates/app/previous/`) for
manual recovery, and the new version takes effect after a user-confirmed restart.
