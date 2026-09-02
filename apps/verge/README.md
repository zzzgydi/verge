# Verge GPUI application

`verge` is the main application crate. Its `verge-gpui` binary is both the GPUI window entry
point and the daemon entry point. Run `scripts/check-rust-workspaces.sh` from the repository root
to validate the workspace.

The crate keeps UI state, typed protocol messages, daemon orchestration, configuration, Mihomo,
and platform integration in separate Rust modules. Views emit typed requests and do not call
Mihomo or macOS APIs directly.

The GUI provides Home, Proxies, Profiles, Connections, Logs, and Settings pages. The daemon mode
owns the native tray, Mihomo lifecycle, persistence, and long-lived system state.

```bash
cargo +1.97.1 check -p verge --all-targets
make dev
```

Run `make dev` from the repository root. It verifies the pinned Mihomo binary,
downloads it to `.cache/mihomo/<target>/mihomo` when necessary, and then starts
the application with `VERGE_MIHOMO_BIN` pointing to that repository cache. Use
`make mihomo` to prepare the binary without starting the GUI.

Build an Apple Silicon `.app` with the pinned sidecar:

```bash
VERGE_MIHOMO_BIN=/absolute/path/to/mihomo \
  apps/verge/scripts/build-macos-app.sh
```

The result is `dist/Verge.app`. The script verifies the sidecar SHA-256 and uses an ad-hoc
signature by default. Set `VERGE_CODESIGN_IDENTITY` to a Developer ID Application identity for a
release candidate. The script also stamps `CFBundleShortVersionString` / `CFBundleVersion` from
`Cargo.toml`, which is what the in-app updater reports as the current version.

## Application self-update

The Settings page can check `github.com/zzzgydi/verge` releases and update the installed
`Verge.app` in place. Release format contract (see `src/application/app_update.rs`
for the full rules):

- Release tag is a semantic version with optional `v` prefix; drafts and pre-releases are skipped.
- The macOS arm64 asset is `Verge-macos-arm64.zip`, containing a single top-level `Verge.app`.
- The sidecar asset `Verge-macos-arm64.zip.sha256` carries the `shasum -a 256` digest line.

An update verifies the zip digest, the bundle structure and its `codesign --verify --deep
--strict` signature, and requires the same signing identity as the running instance before
swapping. The previous bundle is kept under the data directory (`updates/app/previous/`) for
manual recovery, and the new version takes effect after a user-confirmed restart.
