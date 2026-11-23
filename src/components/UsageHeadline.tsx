import type { TimeRange, UsageSummary } from '../types';
import { formatTokens, splitMagnitude } from '../types';

interface UsageHeadlineProps {
  summary: UsageSummary | null;
  range: TimeRange;
  modelCount: number;
  providerCount: number;
}

const RANGE_LABEL: Record<TimeRange, string> = {
  today: 'today',
  week: 'last 7 days',
  month: 'last 30 days',
  all: 'all time',
};

/**
 * The instrument readout.
 *
 * The largest figure on screen is the one number this app exists to answer —
 * how many tokens were used — with the token *mix* directly beneath it. That
 * mix is the part most tools hide: when most of the volume is cache reads, that
 * changes how the number should be read, so it is drawn, not footnoted.
 *
 * The mix bar is normalised over the *reported components*, not over
 * `total_tokens`, on purpose. Providers disagree about whether a cached read is
 * counted inside `input_tokens` or beside it, so `input + output + cached +
 * reasoning` can legitimately exceed the total. Dividing by the component sum
 * keeps every bar at 100% and keeps the caveat explicit instead of inventing a
 * partition the data does not support.
 */
export function UsageHeadline({ summary, range, modelCount, providerCount }: UsageHeadlineProps) {
  const tokens = summary?.total_tokens ?? 0;
  const { value, unit } = splitMagnitude(tokens);

  const input = summary?.input_tokens ?? 0;
  const output = summary?.output_tokens ?? 0;
  const cached = summary?.cached_tokens ?? 0;
  const reasoning = summary?.reasoning_tokens ?? 0;
  const accounted = input + output + cached + reasoning;

  const segments = [
    { key: 'input', label: 'input', value: input, color: 'var(--color-bone-2)' },
    { key: 'output', label: 'output', value: output, color: 'var(--color-needle)' },
    { key: 'cached', label: 'cached', value: cached, color: 'var(--color-verd)' },
    { key: 'reasoning', label: 'reasoning', value: reasoning, color: 'var(--color-brass)' },
  ].filter((s) => s.value > 0);

  return (
    <section className="panel flex flex-col">
      <div className="panel-head">
        <span className="label">Tokens</span>
        <span className="label">{RANGE_LABEL[range]}</span>
      </div>

      <div className="flex flex-1 flex-col justify-between px-4 pb-4 pt-3.5">
        <div>
          <div className="flex items-baseline gap-2">
            <span className="figure text-[clamp(40px,7vw,58px)]">{value}</span>
            {unit && <span className="figure-unit text-[clamp(16px,2.6vw,22px)]">{unit}</span>}
          </div>

          <div className="mt-1.5 flex flex-wrap items-baseline gap-x-2 gap-y-1">
            <span className="mono text-[10.5px] text-bone-2">{tokens.toLocaleString('en-US')}</span>
            <span className="label">tokens</span>
            <span className="text-hair-hi">·</span>
            <span className="mono text-[10.5px] text-bone-2">
              {(summary?.request_count ?? 0).toLocaleString('en-US')}
            </span>
            <span className="label">requests</span>
            {modelCount > 0 && (
              <>
                <span className="text-hair-hi">·</span>
                <span className="mono text-[10.5px] text-bone-2">{modelCount}</span>
                <span className="label">{modelCount === 1 ? 'model' : 'models'}</span>
              </>
            )}
            {providerCount > 0 && (
              <>
                <span className="text-hair-hi">·</span>
                <span className="mono text-[10.5px] text-bone-2">{providerCount}</span>
                <span className="label">
                  {providerCount === 1 ? 'provider' : 'providers'}
                </span>
              </>
            )}
          </div>
        </div>

        <div className="mt-4">
          {segments.length > 0 ? (
            <>
              <div className="mix">
                {segments.map((segment, index) => (
                  <span
                    key={segment.key}
                    className="mix-seg"
                    style={{
                      width: `${(segment.value / Math.max(1, accounted)) * 100}%`,
                      background: segment.color,
                      animationDelay: `${index * 70}ms`,
                    }}
                    title={`${segment.label}: ${segment.value.toLocaleString('en-US')} tokens`}
                  />
                ))}
              </div>

              <div className="mt-2.5 grid grid-cols-2 gap-x-4 gap-y-1 sm:grid-cols-4">
                {segments.map((segment) => (
                  <div key={segment.key} className="flex items-baseline gap-1.5">
                    <span
                      className="mt-[3px] h-[3px] w-[3px] flex-none rounded-full"
                      style={{ background: segment.color }}
                    />
                    <span className="label">{segment.label}</span>
                    <span className="mono ml-auto text-[10.5px] text-bone-2">
                      {formatTokens(segment.value)}
                    </span>
                  </div>
                ))}
              </div>

              {cached > 0 && (
                <p
                  className="caption mt-2"
                  title={
                    'Reported components: ' +
                    accounted.toLocaleString('en-US') +
                    ' tokens across input, output, cached and reasoning'
                  }
                >
                  shares are of reported components — cache reads may already sit
                  inside input.
                </p>
              )}
            </>
          ) : (
            <p className="caption">Nothing recorded in this range yet.</p>
          )}
        </div>
      </div>
    </section>
  );
}
