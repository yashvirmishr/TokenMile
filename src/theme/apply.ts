import type { ResolvedTheme, ThemeColors } from './schema';

type ColorToken = Exclude<keyof ThemeColors, 'series'>;

/**
 * The whole visual contract between a theme pack and the app. These names are
 * stable and are the only CSS variables a community theme needs to know.
 */
const COLOR_VARS: Record<ColorToken, string> = {
  ink: '--color-ink',
  shell: '--color-shell',
  raise: '--color-raise',
  hair: '--color-hair',
  hair_hi: '--color-hair-hi',
  bone: '--color-bone',
  bone_2: '--color-bone-2',
  bone_3: '--color-bone-3',
  needle: '--color-needle',
  verd: '--color-verd',
  brass: '--color-brass',
  slate: '--color-slate',
  alert: '--color-alert',
};

/**
 * Apply a normalized pack to the document root.
 *
 * Values were validated by `normalizeTheme`; `setProperty` only ever receives a
 * custom-property value, never a stylesheet, so a theme cannot inject rules.
 * `data-*` attributes let CSS respond to density and motion without a style
 * recalculation in React.
 */
export function applyTheme(theme: ResolvedTheme): void {
  const root = document.documentElement;

  for (const [token, cssVar] of Object.entries(COLOR_VARS) as [ColorToken, string][]) {
    root.style.setProperty(cssVar, theme.colors[token]);
  }

  theme.colors.series.forEach((color, index) => {
    root.style.setProperty(`--color-series-${index + 1}`, color);
  });

  root.style.setProperty('--font-display', theme.fonts.display);
  root.style.setProperty('--font-label', theme.fonts.label);
  root.style.setProperty('--font-data', theme.fonts.data);

  root.dataset.theme = theme.id;
  root.dataset.themeSource = theme.source;
  root.dataset.appearance = theme.appearance;
  root.dataset.density = theme.density;
  root.dataset.motion = theme.motion ? 'on' : 'off';
  root.style.colorScheme = theme.appearance;
}
