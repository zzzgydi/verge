# GPUI tray macOS spike

This spike answers one Phase 0 question: can official GPUI and `tray-icon` share the macOS main-thread event loop while tray events enter the same typed command path as UI actions?

## Pinned inputs

- Zed/GPUI: `a61e2609c18d459a8f236cbbd0ce5a04a06d28da`, pinned by `Cargo.lock`
- gpui-component: `0e2fb7acdf1d40d4db3d458da3893fe9865e901e`
- `tray-icon`: Cargo `0.21` release line
- Rust: `1.97.1`, matching the pinned Zed revision

The direct GPUI dependencies intentionally use the same unqualified Git source URL as gpui-component. Adding `rev` to only one side makes Cargo treat the same commit as two package sources and produces incompatible duplicate GPUI types. Reproducibility comes from the committed lockfile.

## Current checkpoint

The spike creates a GPUI window and native tray menu on the main thread. Tray menu events become `AppCommand` values. Show activates and focuses the retained GPUI window, Hide hides the application without ending the event loop, and Quit stops the application.

## Run

```bash
cargo test --manifest-path spikes/gpui-tray-macos/Cargo.toml
cargo run --manifest-path spikes/gpui-tray-macos/Cargo.toml
```

The run command is a GUI smoke test and must be checked manually on macOS.
