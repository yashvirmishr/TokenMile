import {
  DEFAULT_THEME,
  normalizeTheme,
  type ResolvedTheme,
} from './schema';

/**
 * Built-in packs are ordinary JSON files under `src/themes/`. Keeping them as
 * files rather than TypeScript objects is intentional: a community author can
 * copy one, edit it, and reload without touching a component or rebuilding the
 * Rust side. Vite bundles them so the default install works offline.
 */
const rawThemes = import.meta.glob<unknown>('../themes/*.json', {
  eager: true,
  import: 'default',
});

function fileName(path: string): string {
  return path.split('/').pop() ?? path;
}

function themeFromModule(path: string, raw: unknown): ResolvedTheme {
  const file = fileName(path);
  const id = file.replace(/\.json$/i, '');
  return normalizeTheme(raw, 'builtin', `builtin:${id}`, file);
}

export const DEFAULT_THEME_KEY = 'builtin:instrument';

export const BUILTIN_THEMES: ResolvedTheme[] = Object.entries(rawThemes)
  .map(([path, raw]) => themeFromModule(path, raw))
  .sort((a, b) => {
    if (a.key === DEFAULT_THEME_KEY) return -1;
    if (b.key === DEFAULT_THEME_KEY) return 1;
    return a.name.localeCompare(b.name);
  });

/** The fallback theme if a build somehow omits `instrument.json`. */
export const FALLBACK_THEME: ResolvedTheme = normalizeTheme(
  DEFAULT_THEME,
  'builtin',
  DEFAULT_THEME_KEY,
  'instrument.json',
);
