# Verge Design Specification

This is the authoritative specification for Verge's interface. Read it before
changing a page or shared component. When another UI document conflicts with this
one, follow this document. Technical integration details live in
[GPUI Kit integration](docs/gpui-kit.md).

## Product And Platform

Verge is a native macOS network utility, not a web dashboard. Aim for the clarity
of Finder, Notes, Reminders and System Settings: precise controls, readable data,
quiet surfaces and predictable keyboard interaction.

The application uses Rust and GPUI Kit. Preserve this architecture, existing
functionality, retained state and typed daemon actions. SwiftUI is a reference
for native interaction patterns, not a requirement to rewrite the application.
Use AppKit only for platform integration that GPUI cannot provide. macOS 15 is
the baseline; do not require a newer appearance API to use the app.

## Visual Direction

- Make typography, alignment and grouping do most of the work. Prefer a separator
  to another enclosing card. One rounded content pane is the window's main surface;
  do not add another decorative wrapper around each page.
- Use opaque, neutral content surfaces. The sidebar and title bar form a continuous
  neutral frame, without a divider between them. Keep brand purple for selected
  controls, links and a few identifying icons, not the entire background.
- No decorative gradients, large shadows, oversized radii or glass content cards.
  Liquid Glass is optional for system-owned navigation and controls on supported
  macOS versions. Do not imitate it with stacked blur or transparent GPUI panels.
- Use the system UI typeface supplied by GPUI on macOS. No bundled display font.
  Use monospaced text for source, addresses and logs when useful.
- Keep useful data visible, not ornamental placeholders. A missing value is
  unknown, not zero. Never draw fabricated traffic or invent successful checks.

## Shared Geometry

All dimensions are logical pixels. `appearance::metrics` owns shared values.

| Element | Size |
| --- | --- |
| Spacing scale | 4, 8, 12, 16, 20, 24, 32 |
| Body / metadata | 14 / 12 |
| Page title / section title | 20 semibold / 14-16 medium |
| Primary metric | 24 medium |
| Page inset / section gap | 20 / 16 |
| Group inset / radius | 16 / 8 |
| Main content pane radius / right and bottom inset | 12 / 8 |
| Control radius | 6 |
| Header / control / compact action height | 32 / 32 / 28 |
| Sidebar row | 36 |
| Proxy row / row gap | 64 / 8 |
| Settings maximum width | 920 |

Keep the 16px rem basis separate from the explicit 14px body size. Window growth
reveals more content rather than increasing type or padding. The supported minimum
window is 960 x 640; also review at 1280 x 800. There is no mobile layout or WebView.

## Window And Navigation

- Preserve native window controls, dragging, menus and fullscreen behavior.
- Keep the top and left window frame connected. All pages sit inside one rounded
  main content surface, with 8px of frame visible on the right and bottom.
- One stable sidebar contains Overview, Proxies, Profiles, Rules, Connections,
  Logs, Intelligence and Settings in that order. Selection uses a quiet tint and
  readable label. Collapsing retains icon alignment and accessible labels.
- Each page uses the shared single-line `PageHeader`: title left, relevant actions
  right. Filters occupy a separate compact row. Headers stay outside body scrolling.
- Do not add a global bottom status bar, an empty footer, fake navigation history
  or a nonfunctional global search field to match a reference image.
- Core state belongs with the current configuration; connection counts belong in
  telemetry and Connections. Show actionable errors near their affected controls
  or in the existing global error region, never hide them in a status bar.

## Overview

The reference image informs information order, not its decorative treatment.

1. A full-width configuration group shows the active profile, source, last update,
   core state/version and Manage Profiles. A divided control region contains the
   system proxy switch, port, TUN, DNS, IPv6 and the mode selector. At minimum width,
   metadata may occupy a separate row; controls must never overlap.
2. Traffic is the main column. Put the chart before rate details, with a clear
   sample-window label, numbered scale and download/upload legend. Use thin,
   shape-preserving smooth curves and subtle area fills over a light grid. Curves
   must pass through the original samples without inventing peaks or negative
   values; never average away real spikes. Download and upload each use one compact
   summary group with a direction icon, rate, total and recent peak. Memory and
   connections remain compact supporting metrics. Sampling stays bounded and only
   invalidates the retained telemetry entity.
3. A narrower supporting column shows observed node latency and current routes.
   Deduplicate nodes, exclude groups, hidden entries and built-in outbounds, and
   explicitly count untested nodes. A previous successful delay is not proof of
   current network health. State the thresholds and that these are latest samples.
   Route names truncate with full-text tooltips, including protocol capabilities;
   show at most two routes and provide View All to open the proxy page.
4. Keep Check Network and Optimize as real, distinct actions. Reports preserve
   incomplete evidence, probe consent, cancel/retry, history and separate write
   confirmation. Do not silently start tests just to populate the overview.

The overview body scrolls as one region. The traffic panel and the combined right
column share top and bottom edges. The node summary keeps its natural height;
Current Proxies fills the remaining space without revealing additional routes.
Keep a compact 480px minimum height; allow both columns to grow together when
wrapped metadata needs more space, without clipping or hiding report controls.
Axis labels stay on one line, with enough measured width for the largest tick.

## Page Patterns

| Page | Pattern |
| --- | --- |
| Proxies | Disclosure groups and compact selectable node rows. Use an explicit checkmark and focus ring; no shadows or thick borders on every idle row. Keep search, sort and delay actions reachable. |
| Profiles | A compact document list with restrained selected treatment, source/update metadata and existing actions. No large decorative icon tiles. |
| Rules | A virtualized data list with thin row separators and compact provider sections. Preserve full-value tooltips and copy actions. |
| Connections | A native-style table with column alignment, useful counts, filtering and reachable close/detail actions. Preserve user column widths. |
| Logs | A flat, two-axis scrollable log viewport. Severity colors the marker, not the entire message. Preserve physical newlines and selectable details. |
| Intelligence | A bounded reading column, restrained welcome actions and a stable composer. Assistant text has no enclosing card; user messages and confirmation groups use subtle grouping. Keep reasoning separate, Stop visible, and changes explicitly confirmed. |
| Settings | Compact category controls and bounded grouped forms with separators. Labels and descriptions align on the left, controls on the right. Long forms scroll without losing Save/Test actions. |

Sheets and dialogs follow the same palette, typography and radii. Reuse GPUI Kit
controls and Root-owned overlay behavior. Do not duplicate global overlay layers.

## Color And Accessibility

- Light and dark themes use the same hierarchy, not identical RGB values.
  Semantic tokens live in `appearance.rs`; do not hard-code colors inside pages.
- Purple identifies download, blue identifies upload. Labels, samples and paths
  must agree. Success, warning and failure colors are reserved for actual states.
- Normal text meets WCAG AA 4.5:1 on normal, selected, hovered and pressed surfaces.
  Selection and status cannot rely on color alone. Keep visible keyboard focus,
  tooltips for truncated values, and accessible names on icon-only actions.
- Preserve Command-1 through Command-8 navigation, Command-R refresh, proxy arrow
  navigation, Enter/Space actions and Command-C copy. Respect reduced motion;
  never animate changing traffic numbers or page layouts for decoration.
- Hover must preserve selection. Incoming delay values and their semantic colors
  update together. Waiting, failed and offline states have explicit text.

## Implementation And Review

Audit the current structure before implementing. Prefer shared components over
page-specific styling, and do not change business logic to simplify a mockup.
Keep inputs, scroll handles and feature entities retained; keep lists virtualized.
Update declared row geometry and hit regions together when dimensions change.

Render the actual GPUI interface, not an HTML approximation. Review all eight pages
in light and dark themes at both window sizes. Include populated, loading, empty,
long-name, error and selected states; exercise scrolling, filters, keyboard paths
and confirmations. Use synthetic preview data without accessing user credentials
or changing system network settings. Report any unverified native behavior.

Before finishing, remove anything still too decorative, too web-like or redundant.
When in doubt, simplify.
