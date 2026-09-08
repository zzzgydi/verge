# Verge

[简体中文](README.zh-CN.md)

Verge is a native macOS proxy client built with Rust, GPUI, and Mihomo. It uses a native UI, a persistent background daemon, typed application commands, and a narrowly scoped privileged helper.

> Development status: active rewrite. The GPUI branch is usable for development and testing, but release packaging is currently limited to macOS 13+ on Apple Silicon.

## Features

- Native GPUI interface with English and Simplified Chinese.
- Persistent menu bar daemon: closing the window does not stop proxying.
- Mihomo profiles from local YAML or remote subscriptions, with optional custom User-Agent and scheduled updates.
- Merge configuration, generated-config preview, validation, atomic writes, health checks, and rollback.
- Persistent network overrides: LAN, IPv6, unified delay, logging, mixed port, DNS, and external controller.
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

## Runtime configuration

Settings → Network saves network overrides independently of subscription YAML. The daemon
combines the selected profile, Merge rules, and these overrides into
`<data_dir>/profiles/runtime-config.yaml`, validates it, and restarts Mihomo. Failed
validation or health checks restore the previous settings and runtime configuration.
The same merge runs when switching or updating profiles. Turning DNS override off restores
the profile's DNS mapping.

Overrides are stored in `<data_dir>/profiles/network-settings.yaml`. On macOS the external
controller secret is stored in Keychain; the runtime YAML containing it is owner-readable
only. The internal controller uses `<data_dir>/control/mihomo.sock` in a private directory,
so disabling the external TCP controller does not disconnect Verge. Changed ports retarget
only enabled system proxy endpoints that still point to Verge; existing recovery records
are preserved. Applying network settings restarts the core and can interrupt active connections.
Network overrides currently stay local and are not included in application settings exports
or encrypted profile backups. TUN remains a separate helper-managed runtime switch.

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

Build a local Release app and ZIP with one command:

```bash
make release
```

This prepares and verifies the pinned Mihomo binary, uses Rust `1.97.1` with
`--release --locked` for the application and helper, strips local symbols from
the bundled Rust binaries, and signs the app for local use. Cargo's original
binaries remain available for profiling. Mihomo keeps its upstream signature
and pinned SHA-256; the helper checksum is recorded after signing.

Outputs:

- `dist/Verge.app`: standalone application with Mihomo and the TUN helper bundled.
- `dist/Verge-macos-arm64.zip`: compressed application bundle.
- `dist/Verge-macos-arm64.zip.sha256`: archive checksum.

The command also reports the app, ZIP, executable, and sidecar sizes. To show
sizes again without building, run `make release-size`. You can copy `Verge.app`
to `/Applications` or open it directly:

```bash
open dist/Verge.app
```

To build and run the bundled executable from the terminal:

```bash
make release-run
# Use separate application data for a test run:
VERGE_DATA_DIR=/tmp/verge-release-test make release-run
```

Before switching between Debug and Release with the same data directory, use
**Quit** in Verge's menu bar to stop the old daemon. Closing only the window
leaves the old executable running, which would invalidate a memory comparison.
The app uses `~/Library/Application Support/Verge` by default, as `make dev` does.
`make release-run` returns when the GUI exits; the daemon still requires **Quit**.

Compare runtime memory using the same profile, page, traffic and observation
period. In Activity Monitor, include both `verge-gpui` processes (GUI and daemon)
and their `mihomo` child. File size and resident memory are different measurements;
a smaller Release binary does not guarantee the same reduction in memory.

A custom Mihomo binary and signing identity remain supported:

```bash
VERGE_MIHOMO_BIN=/absolute/path/to/mihomo \
VERGE_CODESIGN_IDENTITY="Developer ID Application: Example (TEAMID)" \
make release
```

The default signature is ad-hoc and is intended for local testing. The script
verifies the bundle with `codesign --verify --deep --strict`; notarization and
publishing are not included.

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
