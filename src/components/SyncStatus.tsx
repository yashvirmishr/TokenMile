import type { SyncReport } from '../types';
import { formatTokens } from '../types';

interface SyncStatusProps {
  report: SyncReport | null;
  hiddenNotice: boolean;
  onDismissHidden: () => void;
}

/**
 * One line of sync truth under the rail.
 *
 * A full per-provider report is only worth its space when something happened:
 * providers with no new tokens collapse into "no new usage", and the ones that
 * did import something show their delta. Failures never collapse.
 */
export function SyncStatus({ report, hiddenNotice, onDismissHidden }: SyncStatusProps) {
  if (hiddenNotice) {
    return (
      <div className="flex items-center gap-2 border-b border-hair bg-shell/60 px-4 py-1.5">
        <span className="label text-brass">tray</span>
        <span className="text-[11px] text-bone-2">
          Hidden — still running, still syncing.
        </span>
        <button onClick={onDismissHidden} className="ghost-btn ml-auto" title="Dismiss">
          dismiss
        </button>
      </div>
    );
  }

  if (!report) return null;

  const failures = report.providers.filter((p) => !p.ok && !p.skipped);
  const imported = report.providers.filter((p) => p.ok && p.new_tokens > 0);
  const attempted = report.providers.filter((p) => !p.skipped);
  const duration = Math.max(0, report.finished_at - report.started_at);

  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-hair bg-shell/40 px-4 py-1.5">
      <span className="label">{failures.length > 0 ? 'sync incomplete' : 'last pass'}</span>

      {imported.length === 0 && failures.length === 0 ? (
        <span className="text-[11px] text-bone-2">no new usage</span>
      ) : (
        <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
          {imported.map((result) => (
            <span key={result.provider} className="flex items-baseline gap-1.5">
              <span className="label label-bright">{result.name}</span>
              <span className="mono text-[11px] text-verd">
                +{formatTokens(result.new_tokens)}
              </span>
            </span>
          ))}
        </span>
      )}

      {failures.map((result) => (
        <span key={result.provider} className="flex items-baseline gap-1.5 text-alert">
          <span className="label text-alert">{result.name}</span>
          <span className="text-[11px]" title={result.message}>
            {truncate(result.message, 90)}
          </span>
        </span>
      ))}

      <span className="mono ml-auto text-[10px] text-bone-3">
        {attempted.length} provider{attempted.length === 1 ? '' : 's'}
        {duration > 0 ? ` · ${duration}ms` : ''}
      </span>
    </div>
  );
}

function truncate(text: string, max: number): string {
  return text.length <= max ? text : `${text.slice(0, max - 1)}…`;
}
