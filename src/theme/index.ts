/**
 * Public entry point for community UI work.
 *
 * A fork can import `useTheme` here to keep packs working, `normalizeTheme` to
 * validate a pack, or `applyTheme` if it replaces the React shell entirely.
 * The data contract lives in `src/types` and the section map in
 * `src/components/DashboardSections`.
 */
export * from './schema';
export * from './builtins';
export * from './apply';
export * from './useTheme';
