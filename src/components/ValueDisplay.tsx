import type { UsageSummary } from '../types';
import { formatBilling, formatCurrency, formatTokens } from '../types';

interface ValueDisplayProps {
  summary: UsageSummary | null;
}

/**
 * The three money figures, kept strictly apart:
 *
 *   ACTUAL COST      what the user is billed — N/A when no per-token spend exists
 *   REFERENCE VALUE  what the same usage would cost at list rates
 *   FREE VALUE       reference − actual, only when both sides are known
 *
 * `N/A` and `$0.00` are different claims, and the layout says so in words under
 * each figure rather than in a paragraph at the bottom.
 */
export function ValueDisplay({ summary }: ValueDisplayProps) {
  const s = summary;
  const unpriced = s?.unpriced_records ?? 0;
  const total = s?.request_count ?? 0;
  const nothingPriced = total > 0 && unpriced >= total;
  const unbilled = s?.unbilled_records ?? 0;

  const actualKnown = s != null && !nothingPriced && unbilled < total;
  const referenceKnown = s != null && !nothingPriced && (s.reference_value ?? 0) > 0;
  const freeKnown = actualKnown && referenceKnown;

  const marker = unpriced > 0 ? '*' : '';

  return (
    <section className="panel">
      <div className="panel-head">
        <span className="label">Value</span>
        {s != null && total > 0 && (
          <span className="mono text-[10px] text-bone-3">
            {total.toLocaleString('en-US')} record{total === 1 ? '' : 's'}
            {unpriced > 0 ? ` · ${unpriced.toLocaleString('en-US')} unpriced` : ''}
          </span>
        )}
      </div>

      <div className="grid grid-cols-1 divide-y divide-hair sm:grid-cols-3 sm:divide-x sm:divide-y-0">
        <Figure
          label="Actual cost"
          value={actualKnown ? `${formatCurrency(s?.actual_cost ?? 0)}${marker}` : 'N/A'}
          known={actualKnown}
          caption={
            actualKnown
              ? unbilled > 0
                ? `${unbilled.toLocaleString('en-US')} record${
                    unbilled === 1 ? '' : 's'
                  } came from a plan, not a bill`
                : 'what these tokens were billed at'
              : 'no per-token spend: subscription or free access'
          }
          accent="needle"
        />

        <Figure
          label="Reference value"
          value={referenceKnown ? `${formatCurrency(s?.reference_value ?? 0)}${marker}` : 'N/A'}
          known={referenceKnown}
          caption={
            referenceKnown
              ? 'the same usage at list API rates'
              : 'no list price for these models'
          }
          accent="verd"
        />

        <Figure
          label="Free value"
          value={freeKnown ? formatCurrency(s?.free_value ?? 0) : 'N/A'}
          known={freeKnown}
          caption={
            freeKnown
              ? 'reference minus actual'
              : 'needs both sides: a flat fee is not a saving'
          }
          accent="bone"
        />
      </div>

      {(unpriced > 0 || (s?.billing_units ?? 0) > 0) && (
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 border-t border-hair px-4 py-2.5">
          {unpriced > 0 && (
            <>
              <span className="text-needle">*</span>
              <span className="caption">
                {unpriced.toLocaleString('en-US')} of {total.toLocaleString('en-US')} records
                ({formatTokens(s?.unpriced_tokens ?? 0)} tokens) use a model with no price
                entry — excluded, never counted as $0.
              </span>
              <span className="label">[p] pricing</span>
            </>
          )}
          {(s?.billing_units ?? 0) > 0 && (
            <span className="ml-auto flex items-baseline gap-1.5">
              <span className="label">billed units</span>
              <span className="mono text-[11px] text-bone">
                {formatBilling(s?.billing_units, 'ai_credits')}
              </span>
            </span>
          )}
        </div>
      )}
    </section>
  );
}

function Figure({
  label,
  value,
  caption,
  known,
  accent,
}: {
  label: string;
  value: string;
  caption: string;
  known: boolean;
  accent: 'needle' | 'verd' | 'bone';
}) {
  const colour = known
    ? accent === 'needle'
      ? 'text-needle'
      : accent === 'verd'
        ? 'text-verd'
        : 'text-bone'
    : 'text-bone-3';

  return (
    <div className="px-4 py-3.5">
      <div className="label">{label}</div>
      <div className={`figure mt-2 text-[clamp(24px,3.6vw,30px)] ${colour}`}>{value}</div>
      <div className="mt-1.5 text-[10.5px] leading-snug text-bone-3">{caption}</div>
    </div>
  );
}
