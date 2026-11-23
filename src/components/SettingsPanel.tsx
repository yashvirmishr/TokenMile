import type { AppPreferences, FutureProviderView, ProviderStatus, WindowState } from '../types';

interface SettingsPanelProps {
  windowState: WindowState | null;
  prefs: AppPreferences | null;
  providers: ProviderStatus[];
  /** Providers with no supported local source, documented rather than faked. */
  futureProviders: FutureProviderView[];
  pricingVersion: string | null;
  registryModels: number;
  syncIntervalSecs: number;
  onToggleAlwaysOnTop: () => void;
  onToggleProvider: (id: string, enabled: boolean) => void;
  onSetBackfillDays: (days: number) => void;
  onHide: () => void;
  onQuit: () => void;
  onClose: () => void;
  /**
   * Extra sections owned by other parts of the app (appearance/theme controls).
   * Rendered above "Window" so a plugin or a fork can extend settings without
   * editing this file.
   */
  children?: React.ReactNode;
}

/** First-import windows offered in the UI. `0` means all available history. */
const BACKFILL_CHOICES: { days: number; label: string }[] = [
  { days: 7, label: '7 days' },
  { days: 30, label: '30 days' },
  { days: 0, label: 'All available' },
];

/**
 * Settings. Every value is the real state reported by the native window, the
 * preferences file or the provider registry — nothing here is decorative.
 */
export function SettingsPanel({
  windowState,
  prefs,
  providers,
  futureProviders,
  pricingVersion,
  registryModels,
  syncIntervalSecs,
  onToggleAlwaysOnTop,
  onToggleProvider,
  onSetBackfillDays,
  onHide,
  onQuit,
  onClose,
  children,
}: SettingsPanelProps) {
  const alwaysOnTop = windowState?.always_on_top ?? prefs?.always_on_top ?? false;
  const backfillDays = prefs?.backfill_days ?? 30;
  const geometry = windowState
    ? `${windowState.width}×${windowState.height}${
        windowState.x != null && windowState.y != null ? ` at ${windowState.x}, ${windowState.y}` : ''
      }`
    : '—';

  return (
    <section className="panel-open reveal">
      <div className="panel-head">
        <span className="label">Settings</span>
        <button onClick={onClose} className="ghost-btn" title="Close [s]">
          close esc
        </button>
      </div>

      <div className="px-4 pb-4">
        {children && (
          <div className="pt-3">
            <SectionTitle>Appearance</SectionTitle>
            {children}
          </div>
        )}

        <div className="pt-3">
          <SectionTitle>Window</SectionTitle>
          <Row label="Always on top" hint="1">
            <Toggle on={alwaysOnTop} onClick={onToggleAlwaysOnTop} />
          </Row>
          <Row label="Geometry">
            <span className="mono text-[11px] text-bone-2">{geometry}</span>
          </Row>
          <Row label="Preferences file">
            <span className="mono text-[11px] text-bone-2">preferences.json</span>
          </Row>
        </div>

        <div className="pt-4">
          <SectionTitle>Sync</SectionTitle>
          <Row label="Background pass">
            <span className="mono text-[11px] text-bone-2">every {syncIntervalSecs}s</span>
          </Row>
          <Row label="First import window">
            <span className="flex flex-wrap gap-1">
              {BACKFILL_CHOICES.map((choice) => (
                <button
                  key={choice.days}
                  onClick={() => onSetBackfillDays(choice.days)}
                  className={`chip ${backfillDays === choice.days ? 'chip-on' : ''}`}
                  title="How much history a provider imports the first time it syncs"
                >
                  {choice.label}
                </button>
              ))}
            </span>
          </Row>
          <p className="caption pt-2">
            Applies to providers that have not synced yet. History already imported is
            never re-imported.
          </p>
        </div>

        <div className="pt-4">
          <SectionTitle>Providers</SectionTitle>
          {providers.map((provider) => (
            <button
              key={provider.id}
              onClick={() => onToggleProvider(provider.id, !provider.enabled)}
              className="field"
              title={
                provider.connected
                  ? `Synchronize ${provider.name}`
                  : `${provider.name} cannot synchronize: ${provider.status}`
              }
            >
              <Toggle on={provider.enabled} as="span" />
              <span className="flex-1 text-[12px] text-bone">{provider.name}</span>
              <span className="label">{provider.status.toLowerCase()}</span>
            </button>
          ))}
        </div>

        <div className="pt-4">
          <SectionTitle>Pricing</SectionTitle>
          <Row label="Registry">
            <span className="mono text-[11px] text-bone-2">{pricingVersion ?? '—'}</span>
          </Row>
          <Row label="Models">
            <span className="mono text-[11px] text-bone-2">
              {registryModels.toLocaleString('en-US')}
            </span>
          </Row>
        </div>

        {futureProviders.length > 0 && (
          <div className="pt-4">
            <SectionTitle>Not supported yet</SectionTitle>
            {futureProviders.map((provider) => (
              <div key={provider.id} className="border-b border-hair/60 py-2 last:border-b-0">
                <div className="flex items-baseline gap-2">
                  <span className="text-[12px] text-bone">{provider.name}</span>
                  <span className="label text-brass">unsupported</span>
                </div>
                <p className="caption">{provider.reason}</p>
              </div>
            ))}
          </div>
        )}

        <div className="flex gap-2 pt-5">
          <button onClick={onHide} className="chip" title="Hide to tray [h]">
            Hide to tray
          </button>
          <button onClick={onQuit} className="chip" title="Quit [x]">
            Quit
          </button>
        </div>

        <p className="caption pt-3">
          Closing the window hides it to the tray; background sync keeps collecting until
          you quit.
        </p>
      </div>
    </section>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return <div className="label pb-1.5">{children}</div>;
}

function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="field">
      {hint && <span className="key">{hint}</span>}
      <span className="flex-1 text-[12px] text-bone-2">{label}</span>
      {children}
    </div>
  );
}

function Toggle({
  on,
  onClick,
  as = 'button',
}: {
  on: boolean;
  onClick?: () => void;
  as?: 'button' | 'span';
}) {
  const track = (
    <span
      className="relative inline-flex h-[14px] w-[26px] flex-none items-center rounded-full transition-colors"
      style={{ background: on ? 'var(--color-needle)' : 'var(--color-hair-hi)' }}
    >
      <span
        className="absolute h-[10px] w-[10px] rounded-full bg-ink transition-all"
        style={{ left: on ? 14 : 2 }}
      />
    </span>
  );

  if (as === 'span') return track;

  return (
    <button onClick={onClick} role="switch" aria-checked={on} className="flex-none">
      {track}
    </button>
  );
}
