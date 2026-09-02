# Verge

[简体中文](README.zh-CN.md)

Verge is a native macOS proxy client built with Rust, GPUI, and Mihomo. It uses a native UI, a persistent background daemon, typed application commands, and a narrowly scoped privileged helper.

> Development status: active rewrite. The GPUI branch is usable for development and testing, but release packaging is currently limited to macOS 13+ on Apple Silicon.

## Features

- Native GPUI interface with English and Simplified Chinese.
- Persistent menu bar daemon: closing the window does not stop proxying.
- Mihomo profiles from local YAML or remote subscriptions, with optional custom User-Agent and scheduled updates.
- Merge configuration, generated-config preview, validation, atomic writes, health checks, and rollback.
- Rule, Global, and Direct modes; proxy-group selection and delay tests.
- Live traffic, memory, connections, rules, providers, and logs.
- macOS HTTP, HTTPS, SOCKS, PAC, and bypass settings with recovery records.
- TUN lifecycle through a versioned privileged helper.
- Launch at login, global shortcut, notifications, diagnostics, settings import/export, and encrypted backups.
- Verified Mihomo updates and signed application self-updates.

## Architecture

Verge uses one executable in two modes:

```text
Verge.app
  |
  +-- GUI mode: GPUI window and session-only UI state
  |
  +-- --daemon: menu bar, command handling, persistence, Mihomo, system integration
          |
          +-- Unix socket IPC <data_dir>/daemon.sock
          +-- Mihomo sidecar
          +-- privileged helper for TUN
```

The GUI sends typed requests over a local Unix socket. It never calls Mihomo or macOS system APIs directly. The daemon owns the long-lived state, validates commands, manages the sidecar, and streams batched realtime events back to the GUI.

The workspace keeps only boundaries that correspond to independently built or privileged programs:

| Path | Responsibility |
|---|---|
| `apps/verge` | Main application crate: GPUI, daemon, IPC, commands, profiles, Mihomo, and macOS integration |
| `apps/verge-helper` | Privileged TUN helper |
| `crates/verge-helper-protocol` | Small shared protocol between the application and privileged helper |
| `assets/icons` | Application and menu bar icons |
| `assets/branding` | Reusable brand artwork |
| `assets/mihomo/manifest.json` | Pinned Mihomo release and SHA-256 checksums |

The main crate is organized by Rust modules under `apps/verge/src/`. Module privacy keeps UI,
daemon, protocol, application, configuration, Mihomo, and platform responsibilities separated
without requiring a separate Cargo package for every layer.

The earlier React/Tauri/sing-box implementation and the Phase 0 spike projects have been removed.

## Requirements

- macOS 13 or newer.
- Apple Silicon for the current `.app` packaging flow.
- Xcode Command Line Tools.
- Rust `1.97.1` with `rustfmt` and `clippy`.
- A Mihomo `v1.19.26` arm64 binary for release packaging and real contract tests.

```bash
xcode-select --install
rustup toolchain install 1.97.1 --component rustfmt --component clippy
```

## Development

Clone the repository and run the GPUI application:

```bash
git clone git@github.com:zzzgydi/verge.git
cd verge
make dev
```

`make dev` checks the Mihomo binary first. If it is missing or does not match
`assets/mihomo/manifest.json`, the script downloads the asset for the current
platform, verifies both SHA-256 values, installs it under
`.cache/mihomo/<target>/mihomo`, and passes that path to the application through
`VERGE_MIHOMO_BIN`. The repository cache is ignored by Git and does not write to
the user's application data directory. The current development script supports
Apple Silicon macOS.

To prepare Mihomo without starting the application:

```bash
make mihomo
```

The first GUI process starts the same executable with `--daemon` and connects to it over the local socket. Closing the window leaves the daemon and menu bar item running; use **Quit** from the menu bar to stop the complete application.

For isolated development data or a local Mihomo binary:

```bash
VERGE_DATA_DIR=/tmp/verge-dev \
VERGE_MIHOMO_BIN=/absolute/path/to/mihomo \
make dev
```

Useful development overrides:

| Variable | Purpose |
|---|---|
| `VERGE_DATA_DIR` | Override `~/Library/Application Support/Verge` |
| `VERGE_MIHOMO_BIN` | Use a specific Mihomo executable |
| `VERGE_MIHOMO_CACHE_DIR` | Override the repository Mihomo cache directory used by `make dev` |
| `VERGE_MIHOMO_MANIFEST` | Use a different sidecar manifest |
| `VERGE_CONTROLLER` | Set a loopback controller address |
| `VERGE_SECRET` | Set the controller secret |
| `VERGE_NETWORK_SERVICES` | Comma-separated macOS network services |
| `VERGE_HELPER_SOCKET` | Override the privileged-helper socket |

These variables are for development and testing. Do not put real secrets in shell history, committed files, or bug reports.

## Build the macOS application

Download the Mihomo asset named in `assets/mihomo/manifest.json`, verify the archive, and unpack it:

```bash
curl -L \
  https://github.com/MetaCubeX/mihomo/releases/download/v1.19.26/mihomo-darwin-arm64-v1.19.26.gz \
  -o /tmp/mihomo-darwin-arm64-v1.19.26.gz

echo "2d9db5acc7c814a31ff0c04df98b6ac333494ab1ab8e95673ad6e80b28ca6b68  /tmp/mihomo-darwin-arm64-v1.19.26.gz" \
  | shasum -a 256 -c -

gzip -dc /tmp/mihomo-darwin-arm64-v1.19.26.gz > /tmp/mihomo
chmod 755 /tmp/mihomo
```

Build and sign the bundle:

```bash
VERGE_MIHOMO_BIN=/tmp/mihomo \
  apps/verge/scripts/build-macos-app.sh
```

The output is `dist/Verge.app`. The script verifies the unpacked Mihomo SHA-256, builds the GUI and helper in release mode, assembles the bundle, and runs `codesign --verify --deep --strict`. It uses an ad-hoc signature by default.

For a release candidate, provide a Developer ID Application identity:

```bash
VERGE_MIHOMO_BIN=/tmp/mihomo \
VERGE_CODESIGN_IDENTITY="Developer ID Application: Example (TEAMID)" \
  apps/verge/scripts/build-macos-app.sh
```

Notarization and distribution automation are not part of the current build script.

## Testing

Run the repository checks:

```bash
./scripts/check-rust-workspaces.sh
```

The check script tests the unified Rust workspace and runs Clippy for every target with warnings denied.

Real Mihomo contract tests are ignored by default:

```bash
MIHOMO_BIN=/absolute/path/to/mihomo \
  cargo +1.97.1 test -p verge --test mihomo_contract -- --ignored

MIHOMO_BIN=/absolute/path/to/mihomo \
  cargo +1.97.1 test -p verge --test mihomo_runtime_contract -- --ignored
```

The binary must match the pinned executable checksum in `assets/mihomo/manifest.json`.

## Use

1. Build and open `dist/Verge.app`, or run the GPUI target during development.
2. Open **Profiles** and import local YAML content or a remote subscription URL.
3. Activate the profile. Verge validates and materializes a private Mihomo runtime configuration before starting or reloading the core.
4. Use **Overview** or the menu bar to enable the system proxy and choose Rule, Global, or Direct mode.
5. Use **Proxies**, **Rules**, **Connections**, and **Logs** to inspect live state.
6. Use **Settings** for TUN, DNS, IPv6, SOCKS/PAC/bypass, launch at login, shortcuts, updates, backups, and diagnostics.

Installing or removing the privileged helper, enabling TUN, restoring a backup, and replacing the application can change system state. Verge asks for confirmation and macOS may request administrator authorization.

Application data is stored in `~/Library/Application Support/Verge` by default. Logs are written to `logs/verge.log` under that directory. Diagnostic exports redact controller secrets, subscription URLs, authentication headers, and the user home path.

## Platform status

The domain and application layers keep platform APIs behind adapters, but the shipping implementation is currently macOS-first. Windows, Linux, and Intel macOS packages are not available yet. The AI agent described in the rewrite specification is also not implemented in the current codebase.

## License

[GNU General Public License v3.0](LICENSE)
