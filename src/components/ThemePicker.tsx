import { useState, type FormEvent } from 'react';
import type {
  ResolvedTheme,
  ThemeProblem,
  UserThemeFile,
} from '../theme/schema';

interface ThemePickerProps {
  themes: ResolvedTheme[];
  active: ResolvedTheme;
  activeKey: string;
  directory: string | null;
  problems: ThemeProblem[];
  loading: boolean;
  error: string | null;
  onSelect: (key: string) => void;
  onReload: () => void;
  onOpenFolder: () => void;
  onDuplicate: (name: string) => Promise<UserThemeFile | null>;
}

/**
 * Theme chooser for Settings → Appearance.
 *
 * The picker itself is deliberately plain: the preview swatches come from the
 * pack's own tokens, so a community theme is judged by its palette rather than
 * by a screenshot that can go stale. "Start from this pack" writes an editable
 * JSON copy into the themes folder, which is the lowest-friction way to move
 * from using a theme to authoring one.
 */
export function ThemePicker({
  themes,
  active,
  activeKey,
  directory,
  problems,
  loading,
  error,
  onSelect,
  onReload,
  onOpenFolder,
  onDuplicate,
}: ThemePickerProps) {
  const [copyName, setCopyName] = useState('');
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<{ key: string; file: UserThemeFile } | null>(null);

  // Only show the save confirmation while the saved pack is still selected,
  // without an effect that could race the selection update.
  const savedForActive = saved?.key === activeKey ? saved.file : null;

  const submitCopy = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setSaving(true);
    try {
      const result = await onDuplicate(copyName);
      if (result) {
        setSaved({ key: `user:${result.id}`, file: result });
        setCopyName('');
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <div>
      <div className="flex flex-wrap items-center gap-1.5 pb-2">
        <button
          type="button"
          onClick={onOpenFolder}
          className="chip"
          title="Open the folder where community theme JSON files live"
        >
          open themes folder
        </button>
        <button
          type="button"
          onClick={onReload}
          disabled={loading}
          className="chip"
          title="Rescan the themes folder"
        >
          {loading ? 'scanning…' : 'reload'}
        </button>
        <span
          className="mono ml-auto max-w-full truncate text-[9px] text-bone-3"
          title={directory ?? undefined}
        >
          {directory ?? 'built-in packs only'}
        </span>
      </div>

      <div className="theme-grid">
        {themes.map((theme) => {
          const selected = theme.key === activeKey;
          return (
            <button
              key={theme.key}
              type="button"
              onClick={() => onSelect(theme.key)}
              className={`theme-card ${selected ? 'theme-card-on' : ''}`}
              title={theme.description ?? theme.name}
              aria-pressed={selected}
            >
              <span className="theme-swatch" aria-hidden="true">
                {[
                  theme.colors.ink,
                  theme.colors.shell,
                  theme.colors.raise,
                  theme.colors.needle,
                  theme.colors.verd,
                  theme.colors.brass,
                  theme.colors.bone,
                ].map((color, index) => (
                  <span key={index} style={{ background: color }} />
                ))}
              </span>
              <span className="theme-card-body">
                <span className="flex min-w-0 items-baseline gap-1.5">
                  <span className="truncate text-[12px] text-bone">{theme.name}</span>
                  <span className="label ml-auto shrink-0">{theme.source}</span>
                </span>
                {theme.author && (
                  <span className="theme-card-meta truncate">{theme.author}</span>
                )}
                {theme.description && (
                  <span className="theme-card-desc">{theme.description}</span>
                )}
                {selected && <span className="theme-active">active</span>}
              </span>
            </button>
          );
        })}
      </div>

      <p className="caption pt-2">
        A theme pack is JSON: colours, fonts, density, motion and dashboard order.
        Open the folder, copy a built-in pack, or use “start from” to make an
        editable copy without touching the source.
      </p>

      {(problems.length > 0 || error) && (
        <div className="theme-warning">
          {problems.length > 0 && (
            <span className="label text-brass">theme files skipped</span>
          )}
          {error && <p className="mono break-all text-[9.5px] text-alert">{error}</p>}
          {problems.map((problem) => (
            <p key={`${problem.file}:${problem.message}`} className="mono break-all text-[9.5px] text-bone-2">
              {problem.file}: {problem.message}
            </p>
          ))}
        </div>
      )}

      <details className="theme-copy">
        <summary className="label truncate">start from {active.name}</summary>
        <p className="caption pt-1.5">
          Writes an editable copy into the themes folder and selects it. Edit the JSON,
          press reload, and the window updates without a rebuild.
        </p>
        <form onSubmit={submitCopy} className="flex gap-2 pt-2">
          <input
            className="theme-input"
            value={copyName}
            onChange={(event) => setCopyName(event.target.value)}
            placeholder={`copy-of-${active.id}`}
            maxLength={48}
            aria-label="Name for the copied theme"
          />
          <button type="submit" className="chip" disabled={saving}>
            {saving ? 'saving…' : 'save copy'}
          </button>
        </form>
        {savedForActive && (
          <p className="mono break-all pt-1.5 text-[9.5px] text-verd">
            saved {savedForActive.file}
          </p>
        )}
      </details>
    </div>
  );
}
