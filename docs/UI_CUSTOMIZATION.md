# Community UIs and theme packs

FreeTokenMeter is built so the visual layer can be replaced in three increasingly
involved ways:

1. **Pick a built-in pack.** Instrument, Midnight, Paper and Focus ship with the
   app.
2. **Write a JSON theme pack.** Drop a file into the themes folder, press reload,
   and the window restyles without a rebuild. Packs can also reorder or hide
   dashboard sections.
3. **Fork the frontend.** The Rust backend, data hooks and section contract are
   deliberately separate from presentation, so a full UI fork does not need to
   touch providers, SQLite or sync.

The first two are the supported community path. The third is documented at the
end for people who want a different structure entirely.

## Where theme packs live

| Platform | Folder |
|----------|--------|
| Windows | `%APPDATA%\freetokenmeter\themes` |
| macOS | `~/Library/Application Support/freetokenmeter/themes/` |
| Linux | `$XDG_DATA_HOME/freetokenmeter/themes/` (usually `~/.local/share/freetokenmeter/themes/`) |

`Settings → Appearance → open themes folder` creates and opens it. Each `.json`
file in that folder is one pack. Subfolders are not scanned.

The four built-in packs are ordinary JSON files in
[`src/themes/`](../src/themes). Open one, copy it into the user folder, and edit
it. In the app, `Settings → Appearance → start from <pack> → save copy` does the
same thing for you and selects the copy.

## The five-minute workflow

1. Open **Settings** (`s`) → **Appearance**.
2. Under **start from Instrument**, give the copy a name and press **save copy**.
3. Press **open themes folder**.
4. Open the new JSON file in any editor and change a colour or two.
5. Press **reload** in Settings. The window updates immediately.
6. Press **reload** again after each edit. No restart or rebuild is required.

If a file is malformed, the picker lists it under **theme files skipped** and
keeps using the last valid theme. A broken community pack cannot take the meter
down.

## Theme pack schema

A complete pack looks like this:

```json
{
  "schema": 1,
  "id": "neon",
  "name": "Neon",
  "author": "your name",
  "description": "One sentence shown in the picker.",
  "appearance": "dark",
  "density": "comfortable",
  "motion": true,
  "colors": {
    "ink": "#0b0f1a",
    "shell": "#111827",
    "raise": "#1b2436",
    "hair": "#273349",
    "hair_hi": "#3d4f6d",
    "bone": "#eef4ff",
    "bone_2": "#b4c2d9",
    "bone_3": "#7f90aa",
    "needle": "#ff4d8d",
    "verd": "#3dd6b5",
    "brass": "#f5c451",
    "slate": "#89a7c9",
    "alert": "#ff6b5e",
    "series": ["#ff4d8d", "#3dd6b5", "#f5c451", "#8f8cff", "#ff8a5c", "#7fd67a"]
  },
  "fonts": {
    "display": "'Iowan Old Style', Georgia, serif",
    "label": "Bahnschrift, 'Segoe UI', system-ui, sans-serif",
    "data": "'Cascadia Mono', Consolas, monospace"
  },
  "layout": {
    "order": ["headline", "chart", "calendar", "value", "filters", "providers", "activity", "models", "status"],
    "hidden": [],
    "spans": { "headline": 4, "chart": 8 }
  }
}
```

Every field is optional. Missing colours, fonts, density and layout fall back to
the Instrument pack, so a minimal pack is valid:

```json
{
  "name": "Just the accent",
  "colors": { "needle": "#4ea7ff" }
}
```

### Colour tokens

These map one-to-one onto the app's CSS custom properties. They are the whole
visual contract; do not add new CSS variables to a pack.

| Token | Role |
|-------|------|
| `ink` | application background |
| `shell` | panel background |
| `raise` | raised / inset surface |
| `hair`, `hair_hi` | hairlines and borders |
| `bone` | primary text |
| `bone_2` | secondary text |
| `bone_3` | tertiary / micro-label text |
| `needle` | the single accent (selection, live activity, the big figures) |
| `verd` | positive / connected signal |
| `brass` | warning / unsupported signal |
| `slate` | neutral series colour |
| `alert` | error signal |
| `series` | exactly six categorical provider colours |

Accepted colour formats are hex, `rgb()`, `hsl()`, `hwb()`, `lab()`, `lch()`,
`oklab()`, `oklch()`, `color()`, and CSS named colours. Values containing `url()`,
`var()`, braces or semicolons are rejected and replaced by the default. This is a
token document, not a stylesheet.

### Type tokens

- `display` — the serif wordmark and quiet captions.
- `label` — uppercase micro-labels and the large numerals (Bahnschrift/DIN).
- `data` — every aligned figure (monospace).

Font stacks are plain CSS `font-family` values. Keep a generic fallback last
(`serif`, `sans-serif`, or `monospace`). No webfont is downloaded; a pack may
only use fonts already installed on the machine.

`label` is not just a label: the hero figure and large numerals use it. Prefer a
DIN/condensed or monospace face there; a proportional face makes the figure's
width shift as numbers change. `data` is the safe choice for every aligned
figure.

### Behaviour tokens

| Key | Values | Default |
|-----|--------|---------|
| `appearance` | `"dark"`, `"light"` | `"dark"` |
| `density` | `"compact"`, `"comfortable"`, `"roomy"` | `"comfortable"` |
| `motion` | `true`, `false` | `true` |

`appearance` sets the native `color-scheme` and lets CSS adjust the background
grid for light packs. `density` tightens or relaxes rows, panel heads and form
fields. `motion: false` disables reveal, bar and breathing animations.

### Layout

The dashboard is a 12-column grid on wide windows and one column on narrow ones.
Each section has a fixed width, so a pack only chooses order and visibility:

| Section id | Contents | Desktop width |
|------------|----------|---------------|
| `headline` | token total, input/output/cache mix, request count | 5 / 12 |
| `chart` | stacked usage chart over time | 7 / 12 |
| `calendar` | year-long daily token calendar | 12 / 12 |
| `value` | reference value, paid cost, free value | 12 / 12 |
| `filters` | provider filter chips | 12 / 12 |
| `providers` | per-provider table | 7 / 12 |
| `activity` | recent activity feed | 5 / 12 |
| `models` | per-model table | 12 / 12 |
| `status` | provider health and detection states | 12 / 12 |

Rules:

- `order` may be partial. Sections it omits are appended in the default order,
  so a new section added by a later release still appears.
- `filters` may be moved but never hidden. It is the scope of every figure below
  it; hiding it would leave an invisible provider filter active. A `hidden`
  entry for `filters` is ignored.
- `hidden` removes content sections. Hiding every content section is treated as
  a mistake and ignored rather than presenting a filters-only or blank window.
- `spans` optionally overrides a section's desktop width with an integer from 1
  to 12. The defaults are in the table above. Mobile always uses one column.
- Focus is a working example: it hides `calendar`, `activity`, `models` and
  `status`, and uses compact density.

## Sharing a pack

A pack is one JSON file. To publish it:

1. Include a screenshot and the intended appearance (dark/light).
2. Mention any font the pack assumes, since nothing is downloaded.
3. State a licence.
4. Keep the file under 512 KiB (it is tokens, not assets).

Do not bundle JavaScript, CSS or images in a pack. A pack is data and is applied
through validated CSS custom properties only.

## Forking the whole UI

The frontend is a normal React 19 + TypeScript + Tailwind 4 app. You can replace
any or all of it without touching the Rust backend.

### The useful seams

| File | Purpose |
|------|---------|
| `src/App.tsx` | shell: title bar, panels, dashboard composition |
| `src/components/DashboardLayout.tsx` | renders section ids from the active theme in order, with the 12-column grid |
| `src/components/DashboardSections.tsx` | builds the stock section map; spread it and override one entry |
| `src/hooks/useDashboard.ts` | dashboard data, provider filter, time range, sync |
| `src/hooks/useDesktop.ts` | native window state and preferences |
| `src/theme/useTheme.ts` | built-in + user packs, selection, persistence |
| `src/types/index.ts` | the frontend/backend data contract |
| `src/index.css` | the base `@theme` token definitions and component classes |

The backend commands you can rely on are `get_dashboard`, `get_providers`,
`get_preferences`, `sync_all`, `list_user_themes`, `save_user_theme` and
`open_themes_folder`. Their payloads are described in `src/types/index.ts`.

### Swapping a section

`createDefaultSections()` returns the stock dashboard as a section map, so a
fork only has to replace the panel it cares about:

```tsx
const sections = {
  ...createDefaultSections(input),
  headline: <MyOwnHeadline {...} />,
};

<DashboardLayout layout={theme.active.layout} sections={sections} />;
```

`DashboardLayout` takes the map and renders only the ids the active pack keeps,
in its order and spans. A fork can add a new id to `DASHBOARD_SECTIONS` in
`src/theme/schema.ts`, add an entry to its own map, and theme packs can then
order, size and hide the new section with no extra code.

### Swapping the whole dashboard

Keep these three things and the backend contract is safe:

1. Fetch through `useDashboard()` (or the same `invoke('get_dashboard', …)`
   arguments). Do not compute usage or prices in the UI.
2. Preserve `null`. The application rule is that `null` means "not reported",
   never `0`.
3. Keep the theme controller or apply `applyTheme()` yourself if you want
   community packs to keep working.

Then replace the body of `App.tsx` with your layout. The title bar, tray,
background sync, provider detection and SQLite history are independent of it.

### Before sharing a fork

```bash
npm run build                  # TypeScript + production frontend
cd src-tauri && cargo test     # provider/database/pricing/theme tests
```

Check the narrow window (360 px wide), a light pack, a dark pack, and
`prefers-reduced-motion`. The fixed desktop window means a layout that only works
at 1280 px is not finished.

## Validation and safety

- Theme files are read only from the themes folder, only `.json`, only regular
  files, with a 512 KiB limit.
- Invalid JSON and non-object documents are skipped and reported in the picker.
- Theme values are normalized in the frontend; unknown fields are ignored and
  malformed values fall back to the default pack.
- A pack cannot execute code. Colours and fonts are set as CSS custom-property
  values, never as a stylesheet string.
- The selected key is persisted in `preferences.json` as `ui_theme`, validated
  on load, and repaired to `builtin:instrument` if the file it points at is
  removed.
