# Mihomo supervisor spike

This Phase 0 spike isolates process lifecycle management from Tauri, GPUI, system proxy, and privileged helpers.

## Covered

- Explicit core binary and working-directory validation.
- Argument-based process launch without shell interpolation.
- Running, backoff, failed, and stopped states.
- Bounded exponential restart policy.
- Explicit stop and best-effort cleanup on drop.
- Structured lifecycle events for a later application event bus.

## Not covered yet

- Mihomo REST and WebSocket health checks.
- Draining stdout/stderr into bounded log buffers.
- Graceful shutdown before forced termination.
- Signed sidecar manifest and checksum verification.
- Configuration validation and rollback.

## Verify

```bash
cd spikes/mihomo-supervisor
cargo +1.97.1 test
cargo +1.97.1 clippy --all-targets -- -D warnings
```
