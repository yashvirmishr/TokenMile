import { useEffect } from 'react';
import type { TimeRange } from '../types';

interface TimeRangeTabsProps {
  active: TimeRange;
  onChange: (range: TimeRange) => void;
  /** Panels own the number keys while they are open. */
  disabled?: boolean;
}

const ranges: { key: TimeRange; label: string; keyHint: string }[] = [
  { key: 'today', label: 'Today', keyHint: '1' },
  { key: 'week', label: 'Week', keyHint: '2' },
  { key: 'month', label: 'Month', keyHint: '3' },
  { key: 'all', label: 'All', keyHint: '4' },
];

/**
 * Range selector. A single segmented rail rather than four loose buttons: the
 * active range reads as the position of a switch, which is what it is.
 */
export function TimeRangeTabs({ active, onChange, disabled = false }: TimeRangeTabsProps) {
  useEffect(() => {
    if (disabled) return;
    const handler = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      switch (e.key) {
        case '1':
          onChange('today');
          break;
        case '2':
          onChange('week');
          break;
        case '3':
          onChange('month');
          break;
        case '4':
          onChange('all');
          break;
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onChange, disabled]);

  return (
    <div
      className="inline-flex items-center gap-0.5 rounded-[3px] border border-hair bg-shell p-0.5"
      role="tablist"
      aria-label="Time range"
    >
      {ranges.map((r) => (
        <button
          key={r.key}
          role="tab"
          aria-selected={active === r.key}
          onClick={() => onChange(r.key)}
          className={`chip ${active === r.key ? 'chip-on' : ''}`}
          title={`${r.label} [${r.keyHint}]`}
        >
          {r.label}
        </button>
      ))}
    </div>
  );
}
