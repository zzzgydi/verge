# Verge UI guidelines

All GPUI pages share this density and hierarchy. Use Tailwind's spacing and type
scales as a reference, with desktop controls and retained GPUI entities. This is a
native design system; it does not add a web styling dependency.

Use [GPUI Kit](gpui-kit.md) as the application UI entry point. Prefer its styled
components and Base behavior primitives; follow the locked release's APIs and the
[official design guide](https://gpui-kit.com/docs/design-guides).

## Scale

| Role | Logical pixels | Tailwind reference |
| --- | --- | --- |
| Spacing | 4, 8, 12, 16, 20, 24, 32 | 1, 2, 3, 4, 5, 6, 8 |
| Page inset / section gap | 24 / 16 | p-6 / gap-4 |
| Panel inset / compact row inset | 16 / 12 horizontal, 8 vertical | p-4 / px-3 py-2 |
| Metadata | 12 / 16 line height | text-xs |
| Body and controls | 14 / 20 | text-sm |
| Section heading | 16 / 24, medium | text-base |
| Page heading | 24 / 32, semibold | text-2xl |
| Metric value | 24 / 32, medium | text-2xl |
| Control / compact action height | 32 / 28 | h-8 / h-7 |
| Panel / control radius | 12 / 8 | rounded-xl / rounded-lg |
| Icons / selection marker | 16 / 20 | size-4 / size-5 |

`appearance::metrics` owns shared dimensions. The theme uses a 16px rem basis so
GPUI spacing utilities match this scale; the application body explicitly uses
14px text. Do not change the rem basis to adjust body text.

## Layout and content

- Use `PageHeader` for a single 32px title-and-actions row on every page, including
  loading states. No permanent subtitle explaining what the page already names.
- Keep useful state, counts and actions in the header. Filters occupy one separate
  32px toolbar where needed. Empty states explain the next action; settings help
  text explains consequences or non-obvious constraints.
- Use `panel` for grouped content. Avoid nested decorative cards and repeated
  titles. Prefer alignment, typography and restrained borders to extra padding.
- Keep page headers and primary actions outside the scrollable body. Give each
  page and settings category an explicit, distinct scroll-container ID.
- Overview groups the active profile, source, last update, core state and version,
  system proxy, mode and a compact network status row in one full-width panel.
  Below it, current proxies and telemetry share an equal-width, equal-height grid
  row with aligned headings and metric footers. Use a 440px minimum height, 20px
  panel padding and 16px section gaps; smaller windows scroll the body to keep
  these cards readable. Show up to four current proxy groups with member counts,
  selected node protocols and delay; count all eligible groups for the current
  mode in the footer.
  A single route or empty state centers in its space. Keep cumulative traffic
  and recent peaks under their corresponding rates, and use two continuous paths
  scaled to the actual plot width with a subtle horizontal grid. Groups, rules,
  providers, memory and connections use the same compact metric layout.
- Profiles use 12px content padding and an 8px action footer. Settings use a
  bounded 920px form with right-aligned controls and separators instead of nested
  cards. Input-and-action groups must fill their control column.
- Proxy groups and nodes are 64px tall, followed by an 8px gap. A node has two
  lines (name, protocol/capabilities), a centered selection marker, and a 28px
  delay action. Use 12px horizontal padding and a 4px gap between text lines.
  Expanded groups have a 44px toolbar and a 28px left inset shared with node rows.
  Global mode has one persistent search/test/sort toolbar and no group inset.
  Use two columns only when the actual content area is at least 780px wide.
  Expansion and collapse preserve virtualized rows until the animation finishes.
- Rules use 36px rows (56px provider cards plus 8px spacing); connections retain compact table rows;
  logs use 24px physical lines, preserving embedded line breaks. The log viewport
  scrolls in both axes and measures the longest filtered message; never apply
  ellipsis or a fixed single-line height to an entire log event. These data views
  keep their own virtualized scrolling.
- Navigation uses one list of 40px rows without section headings; Intelligence
  follows Logs. Keep icon alignment stable during sidebar animation.
  At 960×640, actions must fit and long names truncate without pushing controls
  out of view. Larger windows add visible data, not larger padding.

## Color and interaction

- Use the app icon's purple (`#761E8D`) as the light theme's primary accent and a
  lighter purple (`#D8A0ED`) in dark mode. Surfaces carry a subtle purple tint;
  primary actions, links, focus rings and selections share the accent. Sidebar
  text stays neutral except for the current page. A selected node uses a filled
  check marker, a contrasting border and a tinted background, never color alone.
- Traffic uses purple for download and teal for upload (`#0C7066` in light mode,
  `#51CFC3` in dark mode). Labels, legend samples, values and paths must agree.
  Keep success, warning and error colors reserved for their semantic states.
- Truncated proxy names and rule fields expose full-text tooltips and copying.
  Proxy groups support keyboard expansion. Nodes expose selection semantics,
  visible focus, arrow navigation across virtualized rows, Enter/Space selection,
  and Command-C copying. Copy and delay actions must not switch the node.
- Use compact icon refresh actions with tooltips and accessible labels.
- Semantic text meets 4.5:1 contrast on normal, selected, hovered and pressed
  surfaces in both themes. Logs color the severity marker, keeping payload text
  neutral and preserving measured physical lines.
- Hover must preserve selection. Delay label and semantic color update in the
  same render, including while hovered or pressed. Pending and failed tests have
  explicit labels; retry stays on the same action.
- Use SVG icons from the existing icon system. Center icons in fixed boxes rather
  than aligning glyphs to text baselines. Preserve native focus and keyboard behavior.
- Keep state in feature entities, emit typed actions, share snapshot data, and
  virtualize long lists. Visual polish must not introduce polling or per-row entities.

## Review

Check every affected page at 960×640 and a wider window, in light and dark themes.
Include populated and empty/loading states, long names, selection, search,
scrolling and a delay result arriving under the pointer. For virtual lists, update
declared row heights together with rendered geometry.

References: [spacing](https://tailwindcss.com/docs/theme#default-theme-variable-reference),
[padding](https://tailwindcss.com/docs/padding),
[typography](https://tailwindcss.com/docs/font-size).
