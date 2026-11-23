import type { ProviderStatus as ProviderStatusType } from '../types';

interface ProviderStatusProps {
  providers: ProviderStatusType[];
}

/**
 * Provider health, told plainly.
 *
 * Every provider gets a state and, when it is not contributing, a reason — an
 * installed tool with no readable source is *unavailable*, not "0 tokens". The
 * technical source kind and last-sync detail stay in the hover title so the
 * panel reads as a status board rather than a debug dump.
 */
export function ProviderStatusPanel({ providers }: ProviderStatusProps) {
  const connected = providers.filter((p) => p.connected).length;

  return (
    <section className="panel">
      <div className="panel-head">
        <span className="label">Provider health</span>
        <span className="mono text-[10px] text-bone-3">
          {connected} of {providers.length} connected
        </span>
      </div>

      <div className="grid grid-cols-1 gap-x-6 px-4 py-1 sm:grid-cols-2">
        {providers.map((p) => (
          <div
            key={p.id}
            className="flex flex-col gap-0.5 border-b border-hair/60 py-2 last:border-b-0 sm:[&:nth-last-child(2)]:border-b-0"
            title={[
              p.source_type + (p.source_version ? ` v${p.source_version}` : ''),
              p.last_sync ? `last sync ${p.last_sync}` : 'never synced',
              p.billing_metric_name ? `metered in ${p.billing_metric_name}` : null,
            ]
              .filter(Boolean)
              .join(' · ')}
          >
            <div className="flex items-baseline gap-2">
              <span
                className="status-dot translate-y-[-1px]"
                style={{ background: stateColour(p.state), opacity: p.enabled ? 1 : 0.35 }}
                aria-hidden
              />
              <span className="text-[12px] text-bone">{p.name}</span>
              <span className="label ml-auto" style={{ color: stateColour(p.state) }}>
                {stateWord(p.state)}
              </span>
              {!p.enabled && <span className="label">off</span>}
            </div>

            {p.detail && <p className="caption pl-3.5">{p.detail}</p>}

            {(p.tokens > 0 || p.records > 0) && (
              <div className="mono flex gap-3 pl-3.5 text-[9.5px] text-bone-3">
                <span>{p.records.toLocaleString('en-US')} records</span>
                <span>{p.tokens.toLocaleString('en-US')} tokens</span>
              </div>
            )}
          </div>
        ))}
      </div>
    </section>
  );
}

function stateColour(state: ProviderStatusType['state']): string {
  switch (state) {
    case 'connected':
      return 'var(--color-verd)';
    case 'disabled':
      return 'var(--color-bone-3)';
    case 'error':
      return 'var(--color-alert)';
    default:
      return 'var(--color-brass)';
  }
}

function stateWord(state: ProviderStatusType['state']): string {
  switch (state) {
    case 'connected':
      return 'connected';
    case 'disabled':
      return 'disabled';
    case 'not_installed':
      return 'not installed';
    case 'usage_unavailable':
      return 'no usage source';
    case 'unsupported':
      return 'unsupported';
    case 'error':
      return 'error';
  }
}
