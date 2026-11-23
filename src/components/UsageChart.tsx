import { useEffect, useMemo, useRef, useState } from 'react';
import type { ProviderStatus, TimeRange, UsageSeries } from '../types';
import { formatTokens, seriesColor } from '../types';

interface UsageChartProps {
  series: UsageSeries | null;
  range: TimeRange;
  providers: ProviderStatus[];
  filterLabel: string;
}

interface Bucket {
  key: string;
  start: Date;
  showOnAxis: boolean;
  total: number;
  requests: number;
  /** Tokens per provider, in catalogue order, zero-filled. */
  slices: { provider: string; tokens: number }[];
}

/**
 * The usage chart: stacked bars, one per bucket, with the provider mix visible in
 * each bar.
 *
 * Design decisions worth keeping:
 *
 * - **The axis is zero-filled.** The backend only returns buckets that have
 *   usage; filling the gaps here means a day with no usage is a visible gap in
 *   the bars rather than a misleadingly contiguous line of data.
 * - **Quiet buckets still get a floor mark** so "no usage" and "no data" never
 *   look the same.
 * - **The readout lives in the panel head.** Hovering replaces the summary
 *   caption with that bucket's exact numbers, so the big figure above never
 *   jumps around while the pointer moves.
 */
export function UsageChart({ series, range, providers, filterLabel }: UsageChartProps) {
  const [hovered, setHovered] = useState<number | null>(null);
  const [width, setWidth] = useState(560);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  // Measure the plot area so SVG text stays at its real size instead of being
  // scaled by a stretched viewBox.
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;

    const update = () => setWidth(Math.max(220, el.clientWidth));
    update();

    const observer = new ResizeObserver(update);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const order = useMemo(() => providers.map((p) => p.id), [providers]);

  const buckets = useMemo(
    () => buildBuckets(series, range, order),
    [series, range, order],
  );

  const height = 148;
  const padding = { top: 16, right: 10, bottom: 18, left: 2 };
  const plotWidth = Math.max(1, width - padding.left - padding.right);
  const plotHeight = height - padding.top - padding.bottom;

  const max = Math.max(1, ...buckets.map((b) => b.total));
  const mean = buckets.length > 0 ? buckets.reduce((sum, b) => sum + b.total, 0) / buckets.length : 0;
  const peak = buckets.reduce<Bucket | null>(
    (best, b) => (best == null || b.total > best.total ? b : best),
    null,
  );

  const slot = plotWidth / Math.max(1, buckets.length);
  const barWidth = Math.max(2, Math.min(18, slot * 0.62));

  const colorFor = (provider: string) => {
    const index = order.indexOf(provider);
    return seriesColor(index >= 0 ? index : 0);
  };

  const active = hovered != null ? buckets[hovered] : null;

  if (series == null || buckets.length === 0) {
    return (
      <section className="panel">
        <div className="panel-head">
          <span className="label">Usage</span>
          <span className="label">{filterLabel}</span>
        </div>
        <div className="px-4 py-10 text-center">
          <p className="caption">No usage in this range.</p>
        </div>
      </section>
    );
  }

  return (
    <section className="panel">
      <div className="panel-head">
        <span className="label">Usage</span>
        <span className="mono text-[10px] text-bone-3">
          {active ? (
            <span className="text-bone">
              <span className="text-bone-3">{bucketTitle(active.start, series.granularity)}</span>
              {'  '}
              {formatTokens(active.total)} tok
              {active.requests > 0 && <span className="text-bone-3"> · {active.requests} req</span>}
            </span>
          ) : (
            <span title={peak ? `Peak ${formatTokens(peak.total)} on ${bucketTitle(peak.start, series.granularity)}` : undefined}>
              avg {formatTokens(Math.round(mean))} / {granularityNoun(series.granularity)}
            </span>
          )}
        </span>
      </div>

      <div ref={wrapRef} className="relative px-3 pt-3 pb-1">
        <svg
          width={width - 24}
          height={height}
          role="img"
          aria-label={`Token usage over time: ${buckets.length} ${granularityNoun(
            series.granularity,
          )}s, average ${formatTokens(Math.round(mean))}`}
          onMouseLeave={() => setHovered(null)}
        >
          <g transform={`translate(${padding.left},${padding.top})`}>
            {/* graticule */}
            {[0.25, 0.5, 0.75].map((f) => (
              <line
                key={f}
                className="chart-grid"
                x1={0}
                x2={plotWidth}
                y1={plotHeight * (1 - f)}
                y2={plotHeight * (1 - f)}
              />
            ))}

            {/* bars */}
            {buckets.map((bucket, i) => {
              const x = i * slot + (slot - barWidth) / 2;
              let cursor = plotHeight;

              if (bucket.total === 0) {
                return (
                  <line
                    key={bucket.key}
                    x1={x}
                    x2={x + barWidth}
                    y1={plotHeight}
                    y2={plotHeight}
                    stroke="rgba(239,231,216,0.14)"
                    strokeWidth={2}
                    strokeLinecap="round"
                  />
                );
              }

              return (
                <g key={bucket.key}>
                  {bucket.slices.map((slice) => {
                    if (slice.tokens <= 0) return null;
                    const h = (slice.tokens / max) * plotHeight;
                    cursor -= h;
                    return (
                      <rect
                        key={slice.provider}
                        className="bar"
                        x={x}
                        y={cursor}
                        width={barWidth}
                        height={Math.max(0.8, h)}
                        opacity={hovered == null || hovered === i ? 0.95 : 0.4}
                        // `fill` as a declaration rather than a presentation
                        // attribute: attribute values must be plain, and a var()
                        // there silently falls back to black in some WebViews.
                        style={{
                          fill: colorFor(slice.provider),
                          animationDelay: `${Math.min(i * 9, 320)}ms`,
                        }}
                      />
                    );
                  })}
                </g>
              );
            })}

            {/* average */}
            {mean > 0 && (
              <line
                className="chart-mean"
                x1={0}
                x2={plotWidth}
                y1={plotHeight - (mean / max) * plotHeight}
                y2={plotHeight - (mean / max) * plotHeight}
              />
            )}

            {/* baseline */}
            <line className="chart-baseline" x1={0} x2={plotWidth} y1={plotHeight} y2={plotHeight} />

            {/* hover band */}
            {hovered != null && (
              <rect
                className="chart-hover-band"
                x={hovered * slot}
                y={0}
                width={slot}
                height={plotHeight}
              />
            )}

            {/* axis */}
            {buckets.map((bucket, i) =>
              bucket.showOnAxis ? (
                <text
                  key={`t-${bucket.key}`}
                  className="chart-tick"
                  x={i * slot + slot / 2}
                  y={plotHeight + 12}
                  textAnchor={i === 0 ? 'start' : i === buckets.length - 1 ? 'end' : 'middle'}
                >
                  {axisLabel(bucket.start, series.granularity)}
                </text>
              ) : null,
            )}

            {/* max scale */}
            <text className="chart-tick" x={0} y={-4} textAnchor="start" opacity={0.75}>
              {formatTokens(max)}
            </text>

            {/* hit areas last so they sit above the bars */}
            {buckets.map((bucket, i) => (
              <rect
                key={`h-${bucket.key}`}
                x={i * slot}
                y={0}
                width={slot}
                height={plotHeight}
                fill="transparent"
                onMouseEnter={() => setHovered(i)}
              />
            ))}
          </g>
        </svg>
      </div>

      <ChartLegend buckets={buckets} providers={providers} />
    </section>
  );
}

/** Provider legend with the range total per provider, so colour has a name. */
function ChartLegend({ buckets, providers }: { buckets: Bucket[]; providers: ProviderStatus[] }) {
  const totals = useMemo(() => {
    const map = new Map<string, number>();
    for (const bucket of buckets) {
      for (const slice of bucket.slices) {
        map.set(slice.provider, (map.get(slice.provider) ?? 0) + slice.tokens);
      }
    }
    return [...map.entries()].sort((a, b) => b[1] - a[1]);
  }, [buckets]);

  if (totals.length === 0) return null;

  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 border-t border-hair px-4 py-2.5">
      {totals.map(([provider, tokens]) => {
        const index = providers.findIndex((p) => p.id === provider);
        const status = providers[index];
        return (
          <span key={provider} className="flex items-center gap-1.5">
            <span
              className="h-[3px] w-2 rounded-full"
              style={{ background: seriesColor(index >= 0 ? index : 0) }}
            />
            <span className="label label-bright">{status?.short_label ?? provider.toUpperCase()}</span>
            <span className="mono text-[10px] text-bone-3">{formatTokens(tokens)}</span>
          </span>
        );
      })}
    </div>
  );
}

// ── Axis building ───────────────────────────────────────────────────────────

function pad2(n: number): string {
  return String(n).padStart(2, '0');
}

function hourKey(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}T${pad2(d.getHours())}`;
}

function dayKey(d: Date): string {
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

function mondayOf(d: Date): Date {
  const copy = new Date(d.getFullYear(), d.getMonth(), d.getDate());
  const offset = (copy.getDay() + 6) % 7; // Monday = 0
  copy.setDate(copy.getDate() - offset);
  return copy;
}

/** How many buckets the axis should hold for a range, before data is applied. */
function bucketCount(range: TimeRange, granularity: string, earliest: Date | null): number {
  switch (granularity) {
    case 'hour':
      return 24;
    case 'day': {
      if (range === 'week') return 7;
      if (range === 'month') return 30;
      // "All time" at day resolution: span the real data, capped for legibility.
      if (earliest == null) return 30;
      const days = Math.ceil((Date.now() - earliest.getTime()) / 86_400_000) + 1;
      return Math.max(7, Math.min(60, days));
    }
    default: {
      if (range === 'month') return 5;
      if (range === 'all') {
        if (earliest == null) return 12;
        const weeks = Math.ceil((Date.now() - earliest.getTime()) / (7 * 86_400_000)) + 1;
        return Math.max(4, Math.min(52, weeks));
      }
      return 12;
    }
  }
}

/**
 * Turn the sparse backend series into a dense, ordered axis.
 *
 * Keys are produced with exactly the formats the SQL used (local time), so an
 * empty bucket and a real zero are the same thing — which is the point.
 */
function buildBuckets(
  series: UsageSeries | null,
  range: TimeRange,
  order: string[],
): Bucket[] {
  if (series == null) return [];

  const byKey = new Map<string, Map<string, { tokens: number; requests: number }>>();
  let earliest: Date | null = null;

  for (const point of series.points) {
    const start = parseKey(point.key, series.granularity);
    if (start == null) continue;
    if (earliest == null || start < earliest) earliest = start;

    let slice = byKey.get(point.key);
    if (!slice) {
      slice = new Map();
      byKey.set(point.key, slice);
    }
    const current = slice.get(point.provider) ?? { tokens: 0, requests: 0 };
    slice.set(point.provider, {
      tokens: current.tokens + point.tokens,
      requests: current.requests + point.requests,
    });
  }

  const now = new Date();
  const count = bucketCount(range, series.granularity, earliest);
  const starts: Date[] = [];

  for (let i = count - 1; i >= 0; i--) {
    const d = new Date(now);
    if (series.granularity === 'hour') {
      d.setMinutes(0, 0, 0);
      d.setHours(d.getHours() - i);
    } else if (series.granularity === 'day') {
      d.setHours(0, 0, 0, 0);
      d.setDate(d.getDate() - i);
    } else {
      const monday = mondayOf(d);
      monday.setDate(monday.getDate() - i * 7);
      d.setTime(monday.getTime());
    }
    starts.push(d);
  }

  // Axis labels: five evenly spaced ticks, always including the ends.
  const tickPositions = new Set<number>();
  if (starts.length <= 5) {
    starts.forEach((_, i) => tickPositions.add(i));
  } else {
    for (let t = 0; t < 5; t++) {
      tickPositions.add(Math.round((t * (starts.length - 1)) / 4));
    }
  }

  return starts.map((start, index) => {
    const key =
      series.granularity === 'hour'
        ? hourKey(start)
        : dayKey(start);

    const data = byKey.get(key);
    const providersInBucket = order.filter((id) => data?.has(id));
    // Providers that are not in the catalogue (e.g. a provider removed from the
    // registry after its data was stored) still deserve to be counted.
    for (const id of data?.keys() ?? []) {
      if (!providersInBucket.includes(id)) providersInBucket.push(id);
    }

    const slices = providersInBucket.map((provider) => ({
      provider,
      tokens: data?.get(provider)?.tokens ?? 0,
    }));

    return {
      key,
      start,
      showOnAxis: tickPositions.has(index),
      total: slices.reduce((sum, s) => sum + s.tokens, 0),
      requests: [...(data?.values() ?? [])].reduce((sum, s) => sum + s.requests, 0),
      slices,
    };
  });
}

function parseKey(key: string, granularity: string): Date | null {
  if (granularity === 'hour') {
    const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2})$/.exec(key);
    if (!match) return null;
    return new Date(
      Number(match[1]),
      Number(match[2]) - 1,
      Number(match[3]),
      Number(match[4]),
    );
  }
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(key);
  if (!match) return null;
  // Constructed from parts on purpose: `new Date('2026-11-23')` parses as UTC
  // midnight, which would shift every bucket west of Greenwich.
  return new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
}

function granularityNoun(granularity: string): string {
  if (granularity === 'hour') return 'hour';
  if (granularity === 'week') return 'week';
  return 'day';
}

function axisLabel(date: Date, granularity: string): string {
  if (granularity === 'hour') return `${pad2(date.getHours())}`;
  return date.toLocaleDateString('en-GB', { day: '2-digit', month: 'short' });
}

function bucketTitle(date: Date, granularity: string): string {
  if (granularity === 'hour') {
    const end = new Date(date.getTime() + 3_600_000);
    return `${date.toLocaleDateString('en-GB', { weekday: 'short', day: '2-digit', month: 'short' })}, ${pad2(
      date.getHours(),
    )}:00–${pad2(end.getHours())}:00`;
  }
  if (granularity === 'week') {
    return `Week of ${date.toLocaleDateString('en-GB', { day: '2-digit', month: 'short' })}`;
  }
  return date.toLocaleDateString('en-GB', { weekday: 'short', day: '2-digit', month: 'short' });
}
