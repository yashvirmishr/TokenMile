import { TimeRangeTabs } from './TimeRangeTabs';
import type { TimeRange } from '../types';

interface TitleBarProps {
  syncing: boolean;
  alwaysOnTop: boolean;
  settingsOpen: boolean;
  pricingOpen: boolean;
  range: TimeRange;
  rangeDisabled: boolean;
  lastSyncAge: string | null;
  onRangeChange: (range: TimeRange) => void;
  onSync: () => void;
  onToggleSettings: () => void;
  onTogglePricing: () => void;
  onHide: () => void;
}

/**
 * The top rail: identity on the left, the range selector in the middle, controls
 * on the right. It is the only chrome the window has — everything else is data.
 */
export function TitleBar({
  syncing,
  alwaysOnTop,
  settingsOpen,
  pricingOpen,
  range,
  rangeDisabled,
  lastSyncAge,
  onRangeChange,
  onSync,
  onToggleSettings,
  onTogglePricing,
  onHide,
}: TitleBarProps) {
  return (
    <header className="relative z-10 flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-hair bg-ink/80 px-3 py-2 backdrop-blur-sm sm:px-4">
      <div className="flex min-w-0 items-center gap-2.5">
        <span className="wordmark">FreeTokenMeter</span>
        <span className="label hidden sm:inline">v0.1.0</span>
        {alwaysOnTop && (
          <span
            className="label text-needle"
            title="Always on top — the meter stays above other windows"
          >
            pinned
          </span>
        )}
      </div>

      <div className="order-3 w-full sm:order-none sm:ml-2 sm:w-auto">
        <TimeRangeTabs active={range} onChange={onRangeChange} disabled={rangeDisabled} />
      </div>

      <div className="ml-auto flex items-center gap-1.5">
        <span
          className="mr-1 hidden items-center gap-1.5 md:flex"
          title={
            lastSyncAge
              ? `Last synchronization pass ${lastSyncAge}`
              : 'Background sync runs every 30 seconds'
          }
        >
          <span className={`live-dot ${syncing ? 'live-dot-busy' : ''}`} />
          <span className="label">{syncing ? 'syncing' : 'live'}</span>
          {lastSyncAge && !syncing && (
            <span className="mono text-[9.5px] text-bone-3">{lastSyncAge}</span>
          )}
        </span>

        <button
          onClick={onSync}
          disabled={syncing}
          className="icon-btn"
          title="Sync every provider now [r]"
          aria-label="Sync now"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 16 16"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.3"
            className={syncing ? 'spin' : undefined}
          >
            <path d="M13.5 8a5.5 5.5 0 1 1-1.7-4" strokeLinecap="round" />
            <path d="M13.6 1.6v3.2h-3.2" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>

        <button
          onClick={onTogglePricing}
          className={`icon-btn ${pricingOpen ? 'icon-btn-on' : ''}`}
          title="Pricing registry [p]"
          aria-label="Pricing registry"
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
            <path d="M8 2.2v11.6" strokeLinecap="round" />
            <path d="M11 5.2c0-1-1.2-1.7-3-1.7s-3 .7-3 1.9c0 1.4 1.4 1.8 3 2.2s3 .8 3 2.2c0 1.2-1.3 1.9-3 1.9s-3-.7-3-1.8" strokeLinecap="round" />
          </svg>
        </button>

        <button
          onClick={onToggleSettings}
          className={`icon-btn ${settingsOpen ? 'icon-btn-on' : ''}`}
          title="Settings [s]"
          aria-label="Settings"
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
            <circle cx="8" cy="8" r="2.1" />
            <path d="M8 1.8v1.6M8 12.6v1.6M14.2 8h-1.6M3.4 8H1.8M12.4 3.6l-1.1 1.1M4.7 11.3l-1.1 1.1M12.4 12.4l-1.1-1.1M4.7 4.7L3.6 3.6" strokeLinecap="round" />
          </svg>
        </button>

        <button
          onClick={onHide}
          className="icon-btn"
          title="Hide to the system tray [h]"
          aria-label="Hide to tray"
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
            <path d="M8 3v7" strokeLinecap="round" />
            <path d="M5.2 7.3 8 10.1l2.8-2.8" strokeLinecap="round" strokeLinejoin="round" />
            <path d="M3 12.4h10" strokeLinecap="round" />
          </svg>
        </button>
      </div>
    </header>
  );
}
