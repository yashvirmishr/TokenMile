import { useEffect, useMemo, useState } from 'react';
import { ALL_PROVIDERS, useDashboard } from './hooks/useDashboard';
import { useDesktop } from './hooks/useDesktop';
import { usePricingRegistry } from './hooks/usePricingRegistry';
import { TitleBar } from './components/TitleBar';
import { ProviderTable } from './components/ProviderTable';
import { ModelUsageTable } from './components/ModelUsageTable';
import { ValueDisplay } from './components/ValueDisplay';
import { ActivityFeed } from './components/ActivityFeed';
import { ProviderStatusPanel } from './components/ProviderStatus';
import { SettingsPanel } from './components/SettingsPanel';
import { PricingPanel } from './components/PricingPanel';
import { SyncStatus } from './components/SyncStatus';
import { UsageCalendar } from './components/UsageCalendar';
import { UsageHeadline } from './components/UsageHeadline';
import { UsageChart } from './components/UsageChart';
import type { TimeRange } from './types';

/** Matches `sync::BACKGROUND_SYNC_INTERVAL_SECS` in the Rust backend. */
const SYNC_INTERVAL_SECS = 30;

const RANGE_LABEL: Record<TimeRange, string> = {
  today: 'today',
  week: 'last 7 days',
  month: 'last 30 days',
  all: 'all time',
};

function App() {
  const dashboard = useDashboard();
  const desktop = useDesktop();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [pricingOpen, setPricingOpen] = useState(false);
  const pricing = usePricingRegistry(pricingOpen);

  const { sync } = dashboard;
  const { toggleAlwaysOnTop, hideToTray, quit } = desktop;

  const providers = dashboard.data?.providers ?? [];

  /**
   * Provider tabs are derived from the providers the backend reports, so adding
   * a provider needs no frontend change. `[q]` clears the filter; the rest are
   * click-only to avoid colliding with the numeric time-range keys.
   */
  const tabs = useMemo(
    () => [
      { id: ALL_PROVIDERS, label: 'All', key: 'q' },
      ...providers.map((provider) => ({
        id: provider.id,
        label: provider.short_label,
        key: '',
      })),
    ],
    [providers],
  );

  // Keyboard shortcuts. Panels capture their keys first so `1` can mean
  // "toggle always on top" inside settings while `1` still means "today"
  // on the dashboard.
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const key = e.key.toLowerCase();

      if (key === 'escape') {
        setSettingsOpen(false);
        setPricingOpen(false);
        return;
      }

      if (settingsOpen) {
        if (key === '1') toggleAlwaysOnTop();
        else if (key === 'h') hideToTray();
        else if (key === 'x') quit();
        else if (key === 's') setSettingsOpen(false);
        return;
      }

      if (pricingOpen) {
        if (key === 'p') setPricingOpen(false);
        return;
      }

      switch (key) {
        case 'r':
          sync();
          break;
        case 's':
          setSettingsOpen(true);
          break;
        case 'p':
          setPricingOpen(true);
          break;
        case 'q':
          dashboard.setProviderFilter(ALL_PROVIDERS);
          break;
      }
    };

    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [settingsOpen, pricingOpen, sync, dashboard, toggleAlwaysOnTop, hideToTray, quit]);

  if (dashboard.loading && !dashboard.data) {
    return (
      <div className="flex h-full w-full flex-col items-center justify-center gap-3">
        <span className="live-dot" />
        <span className="label">measuring</span>
      </div>
    );
  }

  const panelOpen = settingsOpen || pricingOpen;
  const filter = dashboard.providerFilter;
  const providerLabel =
    filter === ALL_PROVIDERS
      ? null
      : providers.find((p) => p.id === filter)?.short_label ?? filter;
  const filterLabel = providerLabel ?? RANGE_LABEL[dashboard.timeRange];

  const summary = dashboard.data?.summary ?? null;
  const providerRows = dashboard.data?.provider_usage ?? [];
  const models = dashboard.data?.model_usage ?? [];

  return (
    <div className="relative z-10 flex h-full w-full flex-col">
      <TitleBar
        syncing={dashboard.syncing}
        alwaysOnTop={desktop.windowState?.always_on_top ?? false}
        settingsOpen={settingsOpen}
        pricingOpen={pricingOpen}
        range={dashboard.timeRange}
        rangeDisabled={panelOpen}
        lastSyncAge={dashboard.lastSync ? ageOf(dashboard.lastSync.finished_at) : null}
        onRangeChange={dashboard.setTimeRange}
        onSync={sync}
        onToggleSettings={() => {
          setPricingOpen(false);
          setSettingsOpen((open) => !open);
        }}
        onTogglePricing={() => {
          setSettingsOpen(false);
          setPricingOpen((open) => !open);
        }}
        onHide={hideToTray}
      />

      <main className="flex-1 overflow-y-auto">
        <div className="mx-auto flex w-full max-w-[1280px] flex-col gap-3 p-3 pb-6 sm:p-4 sm:pb-8">
          {settingsOpen && (
            <SettingsPanel
              windowState={desktop.windowState}
              prefs={desktop.prefs}
              providers={providers}
              futureProviders={desktop.futureProviders}
              pricingVersion={pricing.registry?.version ?? null}
              registryModels={pricing.registry?.registry_models ?? 0}
              syncIntervalSecs={SYNC_INTERVAL_SECS}
              onToggleAlwaysOnTop={toggleAlwaysOnTop}
              onToggleProvider={async (id, enabled) => {
                await desktop.setProviderEnabled(id, enabled);
                await dashboard.refresh();
              }}
              onSetBackfillDays={desktop.setBackfillDays}
              onHide={hideToTray}
              onQuit={quit}
              onClose={() => setSettingsOpen(false)}
            />
          )}

          {pricingOpen && (
            <PricingPanel
              registry={pricing.registry}
              loading={pricing.loading}
              error={pricing.error}
              onReload={pricing.reload}
              onClose={() => setPricingOpen(false)}
            />
          )}

          {(desktop.error || dashboard.error) && (
            <div className="panel border-alert/40 px-4 py-2.5">
              <span className="label text-alert">error</span>
              <p className="mt-1 text-[11px] text-bone-2">{desktop.error ?? dashboard.error}</p>
            </div>
          )}

          <SyncStatus
            report={dashboard.lastSync}
            hiddenNotice={desktop.hiddenNotice}
            onDismissHidden={desktop.dismissHiddenNotice}
          />

          <div
            className="reveal grid gap-3 lg:grid-cols-[minmax(0,0.92fr)_minmax(0,1.08fr)]"
            style={{ ['--reveal-delay' as string]: '0ms' }}
          >
            <UsageHeadline
              summary={summary}
              range={dashboard.timeRange}
              modelCount={models.length}
              providerCount={providerRows.length}
            />
            <UsageChart
              series={dashboard.data?.series ?? null}
              range={dashboard.timeRange}
              providers={providers}
              filterLabel={filterLabel}
            />
          </div>

          <div className="reveal" style={{ ['--reveal-delay' as string]: '40ms' }}>
            <UsageCalendar provider={providerLabel ? filter : null} providerLabel={providerLabel} />
          </div>

          <div className="reveal" style={{ ['--reveal-delay' as string]: '70ms' }}>
            <ValueDisplay summary={summary} />
          </div>

          {/* Scope of the breakdown below: which providers are being read. */}
          <div
            className="reveal flex flex-wrap items-center gap-1.5"
            style={{ ['--reveal-delay' as string]: '110ms' }}
          >
            <span className="label mr-1">scope</span>
            {tabs.map((tab) => (
              <button
                key={tab.id}
                onClick={() => dashboard.setProviderFilter(tab.id)}
                className={`chip ${
                  dashboard.providerFilter === tab.id ? 'chip-filter-on' : ''
                }`}
                title={tab.key ? `Show all providers [${tab.key}]` : `Only ${tab.label}`}
              >
                {tab.label}
              </button>
            ))}
            <span className="mono ml-auto text-[9.5px] text-bone-3">{filterLabel}</span>
          </div>

          <div
            className="reveal grid gap-3 lg:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)]"
            style={{ ['--reveal-delay' as string]: '150ms' }}
          >
            <ProviderTable
              providers={providerRows}
              status={providers}
              rangeLabel={filterLabel}
            />
            <ActivityFeed entries={dashboard.data?.activity ?? []} status={providers} />
          </div>

          <div className="reveal" style={{ ['--reveal-delay' as string]: '190ms' }}>
            <ModelUsageTable models={models} status={providers} />
          </div>

          <div className="reveal" style={{ ['--reveal-delay' as string]: '230ms' }}>
            <ProviderStatusPanel providers={providers} />
          </div>

          <div className="flex flex-wrap items-center gap-x-4 gap-y-1 pt-1">
            <Hint keys="1–4" text="range" />
            <Hint keys="r" text="sync" />
            <Hint keys="p" text="pricing" />
            <Hint keys="s" text="settings" />
            <Hint keys="h" text="tray" />
            <span className="mono ml-auto text-[9px] text-bone-3">
              local only · nothing leaves this machine
            </span>
          </div>
        </div>
      </main>
    </div>
  );
}

function Hint({ keys, text }: { keys: string; text: string }) {
  return (
    <span className="flex items-baseline gap-1.5">
      <span className="key">{keys}</span>
      <span className="label">{text}</span>
    </span>
  );
}

/** "12s", "4m" — the age of the last sync pass. */
function ageOf(finishedAtMs: number): string | null {
  if (!finishedAtMs) return null;
  const seconds = Math.max(0, Math.round((Date.now() - finishedAtMs) / 1000));
  if (seconds < 5) return 'just now';
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  return `${Math.round(minutes / 60)}h ago`;
}

export default App;
