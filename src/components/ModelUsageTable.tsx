import type { ModelUsage as ModelUsageType, ProviderStatus } from '../types';
import { formatBilling, formatCurrency, formatTokens, isUnpriced } from '../types';

interface ModelUsageTableProps {
  models: ModelUsageType[];
  status: ProviderStatus[];
  /** Cap the visible rows; the panel scrolls. */
  limit?: number;
}

const SOURCE_LABELS: Record<string, string> = {
  ftm_override: 'curated',
  litellm: 'litellm',
  litellm_cached: 'litellm',
  provider_reported: 'reported',
  unknown: 'none',
};

/**
 * Ranked model usage.
 *
 * A model with no pricing entry is marked `unpriced` where its cost would be.
 * That is a statement about the data, not a zero — the row keeps its tokens and
 * loses its cost rather than pretending the usage was free. An empty cost cell
 * means "this provider does not bill per token", and the reference column still
 * says what the tokens would have cost.
 */
export function ModelUsageTable({ models, status, limit = 8 }: ModelUsageTableProps) {
  if (models.length === 0) return null;

  const label = (id: string) => status.find((s) => s.id === id)?.short_label ?? id.toUpperCase();
  const visible = models.slice(0, limit);
  const hidden = models.length - visible.length;
  const max = Math.max(1, ...visible.map((m) => m.tokens));

  return (
    <section className="panel">
      <div className="panel-head">
        <span className="label">Models</span>
        <span className="mono text-[10px] text-bone-3">
          {models.length} in range{hidden > 0 ? ` · ${hidden} more` : ''}
        </span>
      </div>

      <div className="grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-3 border-b border-hair px-4 py-1.5 sm:grid-cols-[minmax(0,1fr)_auto_minmax(40px,1.1fr)_auto_auto]">
        <span className="label">Model</span>
        <span className="label text-right">Tokens</span>
        <span className="label hidden sm:block">Share</span>
        <span className="label text-right">Actual</span>
        <span className="label hidden text-right sm:block">Reference</span>
      </div>

      <div className="max-h-[266px] overflow-y-auto">
        {visible.map((m) => {
          const unpriced = isUnpriced(m.pricing_status);
          const billing = formatBilling(m.billing_units, m.billing_unit);
          return (
            <div
              key={`${m.provider}:${m.model}`}
              className="row grid-cols-[minmax(0,1fr)_auto_auto] sm:grid-cols-[minmax(0,1fr)_auto_minmax(40px,1.1fr)_auto_auto]"
            >
              <span className="flex min-w-0 flex-col gap-0.5">
                <span className="flex min-w-0 items-baseline gap-1.5">
                  <span className="label shrink-0">{label(m.provider)}</span>
                  <span
                    className="mono truncate text-[11.5px] text-bone"
                    title={`${m.model} — priced by ${
                      SOURCE_LABELS[m.pricing_source ?? 'unknown'] ?? m.pricing_source
                    }`}
                  >
                    {m.model}
                  </span>
                </span>
                <span className="flex items-baseline gap-1.5">
                  {unpriced ? (
                    <span className="label text-needle">unpriced</span>
                  ) : (
                    <span className="label">
                      {SOURCE_LABELS[m.pricing_source ?? 'unknown'] ?? m.pricing_source}
                    </span>
                  )}
                  <span className="mono text-[9.5px] text-bone-3">
                    {m.percentage.toFixed(1)}% of range
                  </span>
                </span>
              </span>

              <span className="mono text-right text-[11.5px] text-bone-2">
                {formatTokens(m.tokens)}
              </span>

              <span className="hidden items-center sm:flex">
                <span className="measure w-full">
                  <span
                    className="measure-fill"
                    style={{
                      width: `${(m.tokens / max) * 100}%`,
                      background: unpriced ? 'var(--color-hair-hi)' : 'var(--color-bone-2)',
                    }}
                  />
                </span>
              </span>

              <span className="min-w-[62px] text-right">
                {m.actual_cost != null ? (
                  <span className="mono text-[11px] text-needle">
                    {formatCurrency(m.actual_cost)}
                  </span>
                ) : unpriced ? (
                  <span className="mono text-[11px] text-bone-3">—</span>
                ) : (
                  <span className="mono text-[11px] text-bone-3">{billing}</span>
                )}
              </span>

              <span className="mono hidden min-w-[62px] text-right text-[11px] text-bone-3 sm:inline">
                {formatCurrency(m.reference_value)}
              </span>
            </div>
          );
        })}
      </div>
    </section>
  );
}
