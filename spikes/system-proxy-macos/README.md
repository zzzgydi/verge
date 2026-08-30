# macOS system proxy spike

This Phase 0 spike isolates macOS system-proxy behavior behind an injectable command runner. It does not change the current machine during automated tests.

## Covered

- Read current HTTP and HTTPS proxy state with `/usr/sbin/networksetup`.
- Apply a validated host and port without shell interpolation.
- Return the previous snapshot for later restoration.
- Attempt a full rollback when a multi-step apply fails.
- Preserve network-service names as one argument, including spaces.

## Deliberately deferred

- Enumerating and tracking multiple active network services.
- SOCKS and PAC modes.
- Bypass domains.
- Crash-journal persistence and startup recovery.
- Real-machine mutation tests; these require an explicit test service and confirmation.

## Verify

```bash
cd spikes/system-proxy-macos
cargo +1.97.1 test
cargo +1.97.1 clippy --all-targets -- -D warnings
```
