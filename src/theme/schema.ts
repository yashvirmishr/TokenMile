/**
 * Community UI pack schema.
 *
 * A theme pack is deliberately data-only: it maps onto the stable CSS custom
 * properties in `src/index.css` and can reorder the dashboard sections. There
 * is no code in a pack, so a community theme can be dropped into the themes
 * folder without a rebuild and cannot run anything.
 *
 * The normalizers below are defensive rather than prescriptive. Unknown keys
 * are ignored, missing tokens fall back to the built-in instrument palette, and
 * an invalid colour or font never breaks the UI. That is what makes it safe to
 * ship a half-finished theme: it degrades to the default rather than turning the
 * dashboard into an unreadable experiment.
 */

/** Bumped only when the meaning of a token changes, not when one is added. */
export const THEME_SCHEMA_VERSION = 1;

/**
 * Dashboard sections a pack may order or hide. `span` is the section's width on
 * the 12-column desktop grid; mobile always uses one column.
 */
export const DASHBOARD_SECTIONS = [
  { id: 'headline', label: 'Headline', span: 5 },
  { id: 'chart', label: 'Usage chart', span: 7 },
  { id: 'calendar', label: 'Usage calendar', span: 12 },
  { id: 'value', label: 'Value readout', span: 12 },
  { id: 'filters', label: 'Provider filters', span: 12 },
  { id: 'providers', label: 'Provider table', span: 7 },
  { id: 'activity', label: 'Activity feed', span: 5 },
  { id: 'models', label: 'Model table', span: 12 },
  { id: 'status', label: 'Provider status', span: 12 },
] as const;

export type DashboardSectionId = (typeof DASHBOARD_SECTIONS)[number]['id'];

export const DEFAULT_LAYOUT_ORDER: DashboardSectionId[] = DASHBOARD_SECTIONS.map(
  (section) => section.id,
);

export type ThemeSource = 'builtin' | 'user';
export type ThemeDensity = 'compact' | 'comfortable' | 'roomy';
export type ThemeAppearance = 'dark' | 'light';

export interface ThemeColors {
  /** Application background. */
  ink: string;
  /** Panel background. */
  shell: string;
  /** Raised / inset surface. */
  raise: string;
  hair: string;
  hair_hi: string;
  /** Primary text. */
  bone: string;
  bone_2: string;
  bone_3: string;
  /** The single accent. */
  needle: string;
  verd: string;
  brass: string;
  slate: string;
  alert: string;
  /** Exactly six categorical colours for provider series. */
  series: string[];
}

export interface ThemeFonts {
  display: string;
  label: string;
  data: string;
}

export interface ThemeLayout {
  /** Every section, in display order. Normalization always appends missing ones. */
  order: DashboardSectionId[];
  /** Sections a pack chooses not to render. */
  hidden: DashboardSectionId[];
  /** Optional per-section widths on the 12-column desktop grid. */
  spans: Partial<Record<DashboardSectionId, number>>;
}

export interface ThemePack {
  schema: number;
  id: string;
  name: string;
  author?: string;
  description?: string;
  appearance: ThemeAppearance;
  density: ThemeDensity;
  motion: boolean;
  colors: ThemeColors;
  fonts: ThemeFonts;
  layout: ThemeLayout;
}

/** A theme plus the identity it was discovered under. */
export interface ResolvedTheme extends ThemePack {
  /** `builtin:instrument` or `user:my-theme`; this is what preferences store. */
  key: string;
  source: ThemeSource;
  /** File name for a user theme, useful for error messages. */
  file?: string;
}

export interface UserThemeFile {
  id: string;
  file: string;
  path: string;
  contents: string;
}

export interface ThemeProblem {
  file: string;
  message: string;
}

export interface ThemeLibrary {
  directory: string;
  themes: UserThemeFile[];
  problems: ThemeProblem[];
}

export const DEFAULT_COLORS: ThemeColors = {
  ink: '#14120d',
  shell: '#1a1813',
  raise: '#23201a',
  hair: '#2e2a21',
  hair_hi: '#453d30',
  bone: '#efe7d8',
  bone_2: '#bcb2a0',
  bone_3: '#8a8071',
  needle: '#e9613a',
  verd: '#56a295',
  brass: '#c9a227',
  slate: '#6e8ca8',
  alert: '#d95a4a',
  series: ['#e9613a', '#56a295', '#c9a227', '#6e8ca8', '#a8743f', '#7d8c4a'],
};

export const DEFAULT_FONTS: ThemeFonts = {
  display:
    "'Iowan Old Style', 'Palatino Linotype', Palatino, Constantia, Georgia, serif",
  label:
    "Bahnschrift, 'DIN Alternate', 'Segoe UI Variable Display', 'Segoe UI', system-ui, sans-serif",
  data: "'Cascadia Mono', Consolas, 'SF Mono', ui-monospace, monospace",
};

export const DEFAULT_THEME: ThemePack = {
  schema: THEME_SCHEMA_VERSION,
  id: 'instrument',
  name: 'Instrument',
  author: 'FreeTokenMeter',
  description:
    'Warm charcoal, bone figures and a vermilion needle. The instrument the app was designed around.',
  appearance: 'dark',
  density: 'comfortable',
  motion: true,
  colors: DEFAULT_COLORS,
  fonts: DEFAULT_FONTS,
  layout: {
    order: DEFAULT_LAYOUT_ORDER,
    hidden: [],
    spans: {},
  },
};

type RawObject = Record<string, unknown>;

const HEX_COLOR =
  /^#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i;
const COLOR_FUNCTION =
  /^(?:rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch|color)\([0-9a-z.,%\s/+-]+\)$/i;
const NAMED_COLOR = /^[a-z]{3,32}$/i;
const UNSAFE_CSS = /[;{}<>\\]|url\s*\(|expression\s*\(|@import/i;

function asObject(value: unknown): RawObject {
  return value != null && typeof value === 'object' && !Array.isArray(value)
    ? (value as RawObject)
    : {};
}

/** Strip control characters and bound free text shown in the picker. */
function cleanText(value: unknown, maxLength: number): string {
  if (typeof value !== 'string') return '';
  const cleaned = value
    .replace(/[\u0000-\u001f\u007f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  return cleaned.slice(0, maxLength);
}

function normalizeColor(value: unknown, fallback: string): string {
  if (typeof value !== 'string') return fallback;
  const color = value.trim();
  if (!color || color.length > 72 || UNSAFE_CSS.test(color)) return fallback;
  if (HEX_COLOR.test(color) || COLOR_FUNCTION.test(color) || NAMED_COLOR.test(color)) {
    return color;
  }
  return fallback;
}

function normalizeFont(value: unknown, fallback: string): string {
  if (typeof value !== 'string') return fallback;
  const font = value.trim();
  if (!font || font.length > 220 || UNSAFE_CSS.test(font)) return fallback;
  return font;
}

function normalizeDensity(value: unknown): ThemeDensity {
  return value === 'compact' || value === 'roomy' ? value : 'comfortable';
}

function normalizeSections(value: unknown): DashboardSectionId[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<DashboardSectionId>();
  const sections: DashboardSectionId[] = [];

  for (const candidate of value) {
    if (typeof candidate !== 'string') continue;
    const section = DASHBOARD_SECTIONS.find((known) => known.id === candidate);
    if (section && !seen.has(section.id)) {
      seen.add(section.id);
      sections.push(section.id);
    }
  }

  return sections;
}

/**
 * A complete order: what the pack asked for first, then every section it did
 * not mention in the canonical order. A pack can therefore be minimal without
 * accidentally deleting a section that a later release adds.
 */
function completeOrder(value: unknown): DashboardSectionId[] {
  const requested = normalizeSections(value);
  const seen = new Set(requested);
  for (const id of DEFAULT_LAYOUT_ORDER) {
    if (!seen.has(id)) requested.push(id);
  }
  return requested;
}

function normalizeLayout(value: unknown): ThemeLayout {
  const raw = asObject(value);
  // The filter row is the scope of every figure below it. Hiding it would keep
  // silently filtering by a provider the user can no longer see or clear, so a
  // pack may move it but never remove it.
  const hidden = normalizeSections(raw.hidden).filter((id) => id !== 'filters');
  return {
    order: completeOrder(raw.order),
    // Hiding every content section is almost always a mistake; keep at least
    // one so a typo cannot present an empty window (filters alone are not a
    // dashboard).
    hidden: hidden.length >= DASHBOARD_SECTIONS.length - 1 ? [] : hidden,
    spans: normalizeSpans(raw.spans),
  };
}

function normalizeSpans(value: unknown): Partial<Record<DashboardSectionId, number>> {
  const raw = asObject(value);
  const spans: Partial<Record<DashboardSectionId, number>> = {};

  for (const section of DASHBOARD_SECTIONS) {
    const candidate = raw[section.id];
    if (typeof candidate === 'number' && Number.isFinite(candidate)) {
      spans[section.id] = Math.min(12, Math.max(1, Math.round(candidate)));
    }
  }

  return spans;
}

/**
 * Turn any JSON value into a complete, safe theme pack.
 *
 * `key` and `source` are supplied by the loader rather than read from the file,
 * so a theme cannot impersonate a built-in or overwrite another user's key.
 */
export function normalizeTheme(
  value: unknown,
  source: ThemeSource,
  key: string,
  file?: string,
): ResolvedTheme {
  const raw = asObject(value);
  const rawColors = asObject(raw.colors);
  const rawFonts = asObject(raw.fonts);
  const rawSeries = Array.isArray(rawColors.series) ? rawColors.series : [];

  const series = Array.from({ length: 6 }, (_, index) =>
    normalizeColor(
      rawSeries[index],
      DEFAULT_COLORS.series[index] ?? DEFAULT_COLORS.needle,
    ),
  );

  const keyId = key.includes(':') ? key.slice(key.indexOf(':') + 1) : key;
  const id = cleanText(raw.id, 64) || keyId || 'theme';

  return {
    schema: THEME_SCHEMA_VERSION,
    id,
    name: cleanText(raw.name, 64) || id,
    author: cleanText(raw.author, 80) || undefined,
    description: cleanText(raw.description, 240) || undefined,
    appearance: raw.appearance === 'light' ? 'light' : 'dark',
    density: normalizeDensity(raw.density),
    motion: typeof raw.motion === 'boolean' ? raw.motion : true,
    colors: {
      ink: normalizeColor(rawColors.ink, DEFAULT_COLORS.ink),
      shell: normalizeColor(rawColors.shell, DEFAULT_COLORS.shell),
      raise: normalizeColor(rawColors.raise, DEFAULT_COLORS.raise),
      hair: normalizeColor(rawColors.hair, DEFAULT_COLORS.hair),
      hair_hi: normalizeColor(rawColors.hair_hi, DEFAULT_COLORS.hair_hi),
      bone: normalizeColor(rawColors.bone, DEFAULT_COLORS.bone),
      bone_2: normalizeColor(rawColors.bone_2, DEFAULT_COLORS.bone_2),
      bone_3: normalizeColor(rawColors.bone_3, DEFAULT_COLORS.bone_3),
      needle: normalizeColor(rawColors.needle, DEFAULT_COLORS.needle),
      verd: normalizeColor(rawColors.verd, DEFAULT_COLORS.verd),
      brass: normalizeColor(rawColors.brass, DEFAULT_COLORS.brass),
      slate: normalizeColor(rawColors.slate, DEFAULT_COLORS.slate),
      alert: normalizeColor(rawColors.alert, DEFAULT_COLORS.alert),
      series,
    },
    fonts: {
      display: normalizeFont(rawFonts.display, DEFAULT_FONTS.display),
      label: normalizeFont(rawFonts.label, DEFAULT_FONTS.label),
      data: normalizeFont(rawFonts.data, DEFAULT_FONTS.data),
    },
    layout: normalizeLayout(raw.layout),
    key,
    source,
    file,
  };
}

/** Export the portable part of a theme, for "start from this pack". */
export function serializeTheme(theme: ResolvedTheme): string {
  const pack: ThemePack = {
    schema: theme.schema,
    id: theme.id,
    name: theme.name,
    author: theme.author,
    description: theme.description,
    appearance: theme.appearance,
    density: theme.density,
    motion: theme.motion,
    colors: theme.colors,
    fonts: theme.fonts,
    layout: theme.layout,
  };
  return `${JSON.stringify(pack, null, 2)}\n`;
}

export function sectionSpan(id: DashboardSectionId): number {
  return DASHBOARD_SECTIONS.find((section) => section.id === id)?.span ?? 12;
}
