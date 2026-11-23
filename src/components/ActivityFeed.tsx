import type { ActivityEntry, ProviderStatus } from '../types';
import { formatTokens, formatTimestamp, seriesColor } from '../types';

interface ActivityFeedProps {
  entries: ActivityEntry[];
  status: ProviderStatus[];
}

/**
 * The tape: what the meter just measured.
 *
 * Rows are deliberately plain — time, provider, model, tokens — because this is
 * the "is it alive?" panel, not an analytics surface. A coloured tick per
 * provider keeps it scannable without turning it into a rainbow.
 */
export function ActivityFeed({ entries, status }: ActivityFeedProps) {
  const label = (id: string) => status.find((s) => s.id === id)?.short_label ?? id.toUpperCase();
  const colour = (id: string) => {
    const index = status.findIndex((s) => s.id === id);
    return seriesColor(index >= 0 ? index : 0);
  };

  return (
    <section className="panel flex flex-col">
      <div className="panel-head">
        <span className="label">Recent</span>
        <span className="label">{entries.length} event(s)</span>
      </div>

      {entries.length === 0 ? (
        <div className="px-4 py-8 text-center">
          <p className="caption">Nothing measured yet. The next pass will show up here.</p>
        </div>
      ) : (
        <div className="max-h-[262px] overflow-y-auto">
          {entries.slice(0, 12).map((entry, i) => (
            <div
              key={`${entry.timestamp}-${entry.provider}-${i}`}
              className="row grid-cols-[auto_auto_minmax(0,1fr)_auto] items-baseline gap-2.5 px-4 py-1.5"
            >
              <span className="mono text-[9.5px] text-bone-3" title={entry.timestamp}>
                {formatTimestamp(entry.timestamp)}
              </span>
              <span className="flex items-baseline gap-1.5">
                <span
                  className="h-[3px] w-[3px] rounded-full"
                  style={{ background: colour(entry.provider) }}
                />
                <span className="label">{label(entry.provider)}</span>
              </span>
              <span className="mono truncate text-[11px] text-bone-2" title={entry.model}>
                {entry.model}
              </span>
              <span className="mono text-[11px] text-bone">+{formatTokens(entry.tokens)}</span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
