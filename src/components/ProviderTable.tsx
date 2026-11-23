import type { ProviderStatus, ProviderUsage as ProviderUsageType } from '../types';
import { formatBilling, formatCurrency, formatTokens, seriesColor } from '../types';

interface ProviderTableProps {
  providers: ProviderUsageType[];
  status: ProviderStatus[];
  /** Shown when the selected range has no rows at all. */
  rangeLabel: string;
}

/**
 * Where the tokens went, as a ranked list with a proportional measure per row.
 *
 * The measure is the point: percentages in a table are hard to compare, a shared
 * bar is not. ACTUAL stays `N/A` for plan usage, and a credit-metered provider
 * shows its units instead of a dollar figure — those are different claims.
 */
export function ProviderTable({ providers, status, rangeLabel }: ProviderTableProps) {
  const label = (id: string) => status.find((s) => s.id === id)?.short_label ?? id.toUpperCase();
  const colour = (id: string) => {
    const index = status.findIndex((s) => s.id === id);
    return seriesColor(index >= 0 ? index : 0);
  };

  const max = Math.max(1, ...providers.map((p) => p.tokens));

  return (
    <section className="panel">
      <div className="panel-head">
        <span className="label">Providers</span>
        <span className="label">{rangeLabel}</span>
      </div>

      {providers.length === 0 ? (
        <div className="px-4 py-8 text-center">
          <p className="caption">No provider recorded usage in this range.</p>
        </div>
      ) : (
        <>
          <div className="grid grid-cols-[minmax(78px,1fr)_auto_minmax(44px,1.6fr)_auto] items-center gap-3 border-b border-hair px-4 py-1.5 sm:grid-cols-[minmax(90px,1fr)_auto_minmax(44px,1.8fr)_auto_auto]">
            <span className="label">Provider</span>
            <span className="label text-right">Tokens</span>
            <span className="label">Share</span>
            <span className="label text-right">Actual</span>
            <span className="label hidden text-right sm:block">Reference</span>
          </div>

          {providers.map((p) => {
            const billing = formatBilling(p.billing_units, p.billing_unit);
            const meta = status.find((s) => s.id === p.provider);
            return (
              <div
                key={p.provider}
                className="row grid-cols-[minmax(78px,1fr)_auto_minmax(44px,1.6fr)_auto] sm:grid-cols-[minmax(90px,1fr)_auto_minmax(44px,1.8fr)_auto_auto]"
              >
                <span className="flex min-w-0 items-center gap-2">
                  <span
                    className="status-dot"
                    style={{ background: colour(p.provider) }}
                    aria-hidden
                  />
                  <span className="flex min-w-0 flex-col">
                    <span className="label label-bright truncate">{label(p.provider)}</span>
                    <span className="mono text-[9.5px] text-bone-3">
                      {p.records.toLocaleString('en-US')} req
                      {meta && !meta.connected ? ' · offline' : ''}
                    </span>
                  </span>
                </span>

                <span className="mono text-right text-[12px] text-bone">
                  {formatTokens(p.tokens)}
                </span>

                <span className="flex items-center gap-2">
                  <span className="measure w-full">
                    <span
                      className="measure-fill"
                      style={{
                        width: `${(p.tokens / max) * 100}%`,
                        background: colour(p.provider),
                      }}
                    />
                  </span>
                  <span className="mono w-9 text-right text-[9.5px] text-bone-3">
                    {p.percentage.toFixed(0)}%
                  </span>
                </span>

                <span
                  className={`mono text-right text-[12px] ${
                    p.actual_cost == null && billing === '—' ? 'text-bone-3' : 'text-needle'
                  }`}
                  title={p.billing_metric_name ?? 'Actual cost'}
                >
                  {p.actual_cost != null
                    ? formatCurrency(p.actual_cost)
                    : billing !== '—'
                      ? billing
                      : 'N/A'}
                </span>

                <span className="mono hidden text-right text-[12px] text-bone-3 sm:block">
                  {formatCurrency(p.reference_value)}
                </span>
              </div>
            );
          })}
        </>
      )}
    </section>
  );
}
