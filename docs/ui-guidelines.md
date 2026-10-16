# Verge UI Implementation Notes

[DESIGN.md](../DESIGN.md) is the authoritative product design specification.
Read it before changing any page. These notes preserve implementation constraints,
not a second visual system. [GPUI Kit integration](gpui-kit.md) describes the locked
component APIs and platform setup.

## Retained State And Geometry

- `appearance::metrics` defines common dimensions; `appearance.rs` defines semantic
  colors for both themes. Use the shared `PageHeader`, `panel` and settings
  row components. Do not introduce per-page palettes or a global status bar.
- Every page owns its scrollable body with a distinct container ID. Headers and
  important actions stay fixed. Preserve input entities and scroll handles across
  renders. Root mounts sheets, dialogs and notifications exactly once.
- Proxy rows remain 64px tall plus an 8px gap, with 12px horizontal padding. Expanded
  groups have a 44px toolbar and 28px indentation; global mode has no indentation.
  Use two columns only when the actual content area is at least 780px wide.
  Keep virtual rows alive during expansion/collapse animation.
- Rules use 36px rows and 56px provider sections plus 8px spacing. Connections retain
  measured column widths. Logs use 24px physical lines with two-axis scrolling and
  measured message widths; never truncate a complete multi-line log event to one row.
- Delay result labels and semantic colors update in the same render, including
  hovered and pressed states. Keep explicit waiting/failure labels and retry actions.
  Copy and delay actions must not also switch the node.

## Overview Evidence

- The chart uses 40 bounded samples, scaled to its actual width. Do not call this
  one minute: the stream may reconnect or sample at a different rate. Download is
  purple and upload is blue. Missing values remain unknown; cumulative counters
  and peaks stay associated with the correct rate.
- Use shape-preserving cubic interpolation, not moving averages or an unconstrained
  spline. Draw both translucent fills before either stroke. Keep numbered ticks
  in one unit and use rounded scale bounds with headroom above the largest sample.
  Measure the longest tick at the rendered font size; never wrap numeric ticks.
- Current Proxies shows at most two rows and links to the full proxy page. The
  combined supporting column and traffic panel share the same height; extra routes
  must not make the overview taller. Use a minimum height, not a fixed cap, so
  wrapped metadata cannot push the report action outside its panel. Custom-content
  buttons need an explicit accessible name.
- Node latency counts unique visible endpoints, not group memberships or built-in
  outbounds. Below 200ms, at least 200ms, failed and untested are separate categories.
  Explicit delay results override snapshot values; a newer test failure supersedes
  an older successful delay. These are latest observations,
  not a new health check. Do not issue backend requests from render.
- Check Network and Optimize preserve distinct entry points and the independent
  report entity. Local checks remain available without probe consent. Active probes
  require opt-in because Mihomo shared health records may affect automatic routing.
- Reports lead with a specific verdict and next action. Group latest probe results
  by node and target. Technical evidence is collapsible; history and observation
  settings are separate tabs. Incomplete checks cannot become successful verdicts.
- Comparison requires explicit group selection. Proposed changes show before/after
  and exit-IP impact and require confirmation. Keep errors visible across tabs and
  retain terminal check failures in historical reports. A sole report can be cleared.

## Intelligence And Forms

- Provider reasoning stays separate from answers: expand while thinking, collapse
  on the first answer text, and retain manual reopening during subsequent chunks.
  Providers without reasoning fields get no synthetic reasoning section.
- Keep Stop visible during reasoning and tool calls. Welcome shortcuts start real
  requests, preserve drafts and never bypass confirmation of configuration changes.
- AI settings has a multiline appended system prompt, Restore Default and a fixed
  Save/Test footer. Other settings use bounded forms and row separators rather than
  nested cards. Input/action groups fill their control column.

## Validation

Run `./scripts/check-rust-workspaces.sh`, format and locale checks. Verify contrast,
keyboard selection, pending operations, long values, scrolling and minimum-width
hit regions. Capture production GPUI views with the native Metal renderer on the
main thread using `HeadlessAppContext::with_platform` and `capture_screenshot`.
Synthetic screenshots are visual evidence, not a test of real network operations
or native accessibility integration. Follow the complete matrix in `DESIGN.md`.
