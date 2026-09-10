# GPUI Kit integration

Verge uses the stable `gpui-kit 0.6.1` release, published on September 9, 2026.
`Cargo.lock` resolves its GPUI implementation to the matching `gpui-pre 0.3.4`
family. Use the [official documentation](https://gpui-kit.com/docs/) together
with the source of the locked release when checking signatures; website examples
may include changes newer than the release.

## Dependencies and imports

The application has one UI dependency:

```toml
[dependencies]
gpui-kit = "0.6.1"

[dev-dependencies]
gpui-kit = { version = "0.6.1", features = ["test-support"] }
```

Use the facade throughout application code:

| Purpose | Import / entry point |
| --- | --- |
| GPUI entities, windows, elements, actions, macros | `gpui_kit::{...}` |
| Styled controls, theme, Root, scrolling | `gpui_kit::component::{...}` |
| Behavior primitives and semantic tokens | `gpui_kit::base::{...}` |
| Default component icon assets | `gpui_kit::assets::Assets` |
| Platform APIs | `gpui_kit::platform::{...}` |
| Bootstrap | `gpui_kit::application()` and `gpui_kit::init(cx)` |
| UI tests | `#[gpui_kit::test]` and explicit test type imports |

Do not add a separate Zed Git GPUI dependency or alias an old component package.
The Kit dependency selects compatible GPUI types. The lockfile still contains
`gpui-component` and `gpui-component-macros`: these are the styled layer and its
macros inside GPUI Kit, both at 0.6.1, not leftover application dependencies.

Default features include components and the standard icon assets. Verge's custom
asset source adds its logo and falls back to `gpui_kit::assets::Assets`; it does
not register the complete `AllAssets` icon catalog. `test-support` is enabled only
for development tests. No JavaScript shell or webview is enabled.

## Window and state ownership

- Initialize Kit before constructing any component-backed views. Each window
  keeps one `Root`; the main view renders its sheet, dialog and notification layers.
- Retain inputs, editors, subscriptions, scroll handles and page entities across
  renders. `render` describes the current frame and does not perform backend IO.
- Continue sending typed requests to the daemon. Keep the existing feature modules
  and helper privilege boundary; a dependency migration does not require a crate
  for each page.
- Read semantic colors from the component theme. After applying Verge's palette
  through `Theme::global_mut`, call `Theme::sync_base(cx)` so Base text, scrolling
  and other shared facilities receive the same theme.
- Follow [Verge's UI guidelines](ui-guidelines.md) for product density and layout.

## Validation and platform scope

Run `./scripts/check-rust-workspaces.sh` for the application, UI interaction tests,
helper and vendored dependency tests, followed by strict Clippy. Test modules
should import Kit types explicitly: a glob import also brings in its `test` macro
and can shadow Rust's ordinary `#[test]`.

For pixels on macOS, Kit exposes `HeadlessAppContext::with_platform` and
`capture_screenshot` with the Metal renderer. Initialize AppKit on the main thread;
an ordinary Rust test harness uses worker threads. Use production views and assets,
and keep screenshot checks separate from event/state assertions.

The current [installation guide](https://gpui-kit.com/docs/installation) specifies
macOS 15+ for development. This migration was built and tested on macOS 15.7.9
Apple Silicon. The existing bundle minimum remains 13.0; macOS 13/14 runtime
compatibility has not been revalidated and must not be inferred from successful
compilation on macOS 15.

References: [setup](https://gpui-kit.com/docs/getting-started),
[assets](https://gpui-kit.com/docs/assets),
[coding](https://gpui-kit.com/docs/coding-guides),
[design](https://gpui-kit.com/docs/design-guides),
[testing](https://gpui-kit.com/docs/test).
