import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { applyTheme } from './apply';
import { BUILTIN_THEMES, DEFAULT_THEME_KEY, FALLBACK_THEME } from './builtins';
import {
  normalizeTheme,
  serializeTheme,
  type ResolvedTheme,
  type ThemeLibrary,
  type ThemeProblem,
  type UserThemeFile,
} from './schema';

const STORAGE_KEY = 'ftm.ui-theme';

interface PreferencesWithTheme {
  ui_theme?: string;
}

/** `npm run dev` in a browser has no IPC; built-in packs should still work. */
function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

function readStoredThemeKey(): string | null {
  try {
    return typeof localStorage === 'undefined'
      ? null
      : localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

function storeThemeKey(key: string): void {
  try {
    if (typeof localStorage !== 'undefined') {
      localStorage.setItem(STORAGE_KEY, key);
    }
  } catch {
    // A locked-down webview can refuse storage; the native preference below is
    // the real source of truth, so this is only a fast-start cache.
  }
}

function slugify(value: string): string {
  return (
    value
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 48) || 'community-theme'
  );
}

export interface ThemeController {
  themes: ResolvedTheme[];
  active: ResolvedTheme;
  activeKey: string;
  directory: string | null;
  problems: ThemeProblem[];
  loading: boolean;
  error: string | null;
  selectTheme: (key: string) => void;
  reload: () => Promise<void>;
  openFolder: () => Promise<void>;
  duplicateActiveTheme: (name: string) => Promise<UserThemeFile | null>;
}

/**
 * Load built-in and community theme packs, apply the active one and persist the
 * choice.
 *
 * Preferences are the source of truth; `localStorage` only prevents a flash of
 * the default theme before the IPC round-trip. If a saved user theme has been
 * deleted, the controller falls back to the instrument pack and repairs the
 * saved key instead of leaving the app unthemed.
 */
export function useTheme(): ThemeController {
  const [themes, setThemes] = useState<ResolvedTheme[]>(BUILTIN_THEMES);
  const [activeKey, setActiveKeyState] = useState<string>(
    () => readStoredThemeKey() ?? DEFAULT_THEME_KEY,
  );
  const [directory, setDirectory] = useState<string | null>(null);
  const [problems, setProblems] = useState<ThemeProblem[]>([]);
  const [loading, setLoading] = useState(false);
  const [userThemesLoaded, setUserThemesLoaded] = useState(false);
  const [userThemesTrusted, setUserThemesTrusted] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const hydrated = useRef(false);
  const pendingSelection = useRef<string | null>(null);

  const loadUserThemes = useCallback(async () => {
    setLoading(true);
    if (!isTauriRuntime()) {
      setThemes(BUILTIN_THEMES);
      setDirectory(null);
      setProblems([]);
      setError(null);
      setLoading(false);
      setUserThemesLoaded(true);
      setUserThemesTrusted(false);
      return;
    }
    try {
      const library = await invoke<ThemeLibrary>('list_user_themes');
      const parsed: ResolvedTheme[] = [];
      const parseProblems = [...library.problems];

      for (const file of library.themes) {
        try {
          const raw = JSON.parse(file.contents) as unknown;
          parsed.push(
            normalizeTheme(raw, 'user', `user:${file.id}`, file.file),
          );
        } catch (err) {
          parseProblems.push({
            file: file.file,
            message: String(err),
          });
        }
      }

      parsed.sort((a, b) => a.name.localeCompare(b.name));
      setThemes([...BUILTIN_THEMES, ...parsed]);
      setDirectory(library.directory);
      setProblems(parseProblems);
      setError(null);
      setUserThemesTrusted(true);
    } catch (err) {
      // Browser-only dev or an unavailable command: built-ins still work. Do
      // not trust this scan to delete a saved preference; the folder may only
      // be temporarily unreadable.
      setThemes(BUILTIN_THEMES);
      setDirectory(null);
      setProblems([]);
      setError(String(err));
      setUserThemesTrusted(false);
    } finally {
      setLoading(false);
      setUserThemesLoaded(true);
    }
  }, []);

  useEffect(() => {
    void loadUserThemes();
  }, [loadUserThemes]);

  useEffect(() => {
    let live = true;
    if (!isTauriRuntime()) {
      hydrated.current = true;
      return;
    }
    invoke<PreferencesWithTheme>('get_preferences')
      .then((prefs) => {
        if (!live) return;
        hydrated.current = true;
        const saved = pendingSelection.current ?? prefs.ui_theme;
        if (saved) {
          setActiveKeyState(saved);
          storeThemeKey(saved);
        }
      })
      .catch(() => {
        hydrated.current = true;
      });
    return () => {
      live = false;
    };
  }, []);

  const active = useMemo(
    () =>
      themes.find((theme) => theme.key === activeKey) ??
      themes.find((theme) => theme.key === DEFAULT_THEME_KEY) ??
      themes[0] ??
      FALLBACK_THEME,
    [themes, activeKey],
  );

  useLayoutEffect(() => {
    applyTheme(active);
  }, [active]);

  // A deleted user theme must not leave a stale preference behind. Wait until
  // the user folder has been scanned so a saved `user:` key is not mistaken for
  // missing during the first built-in-only render.
  useEffect(() => {
    if (!hydrated.current || !userThemesLoaded || !userThemesTrusted) return;
    if (themes.some((theme) => theme.key === activeKey)) return;
    setActiveKeyState(DEFAULT_THEME_KEY);
    storeThemeKey(DEFAULT_THEME_KEY);
    if (isTauriRuntime()) {
      void invoke('set_ui_theme', { theme: DEFAULT_THEME_KEY }).catch(() => {
        // The preference will be repaired on the next successful selection.
      });
    }
  }, [themes, activeKey, userThemesLoaded, userThemesTrusted]);

  const selectTheme = useCallback((key: string) => {
    if (!hydrated.current) pendingSelection.current = key;
    setActiveKeyState(key);
    storeThemeKey(key);
    if (!isTauriRuntime()) return;
    void invoke('set_ui_theme', { theme: key })
      .then(() => setError(null))
      .catch((err) => {
        setError(String(err));
      });
  }, []);

  const openFolder = useCallback(async () => {
    if (!isTauriRuntime()) {
      setError('The themes folder is only available in the desktop app.');
      return;
    }
    try {
      const path = await invoke<string>('open_themes_folder');
      setDirectory(path);
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  }, []);

  /**
   * Write the active pack into the user themes folder so it becomes editable.
   * This is the "fork without forking the repo" button: a JSON file appears,
   * the folder opens, and one reload brings the edited copy back into the UI.
   */
  const duplicateActiveTheme = useCallback(
    async (name: string): Promise<UserThemeFile | null> => {
      if (!isTauriRuntime()) {
        setError('Saving a theme copy is only available in the desktop app.');
        return null;
      }

      const label = name.trim() || `Copy of ${active.name}`;
      const copy: ResolvedTheme = {
        ...active,
        id: slugify(label),
        name: label,
        // A copy is the author's, not FreeTokenMeter's.
        author: undefined,
      };

      try {
        const saved = await invoke<UserThemeFile>('save_user_theme', {
          name: label,
          contents: serializeTheme(copy),
        });
        await loadUserThemes();
        selectTheme(`user:${saved.id}`);
        setError(null);
        return saved;
      } catch (err) {
        setError(String(err));
        return null;
      }
    },
    [active, loadUserThemes, selectTheme],
  );

  return {
    themes,
    active,
    activeKey,
    directory,
    problems,
    loading,
    error,
    selectTheme,
    reload: loadUserThemes,
    openFolder,
    duplicateActiveTheme,
  };
}
