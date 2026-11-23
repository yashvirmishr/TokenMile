import { useMemo, useState, type CSSProperties } from 'react';
import { useCalendar } from '../hooks/useCalendar';
import { formatTokens } from '../types';

interface UsageCalendarProps {
  /** Canonical provider id, or null for every provider. */
  provider: string | null;
  /**
   * Short label of the selected provider, or null for all of them. This is not
   * the range label: the calendar always spans a year, so showing "today" in its
   * caption would describe data it does not contain.
   */
  providerLabel: string | null;
}

/** Weeks drawn: 53 columns covers a year and always ends in the current week. */
const WEEKS = 53;
const DAYS = WEEKS * 7;
const GAP = 2;

/** Four intensity steps above "nothing recorded". */
const LEVELS = 4;
const LEVEL_PERCENT = [0, 22, 44, 70, 100];

/**
 * Cells are sized by the panel, not by a constant: 53 columns of `1fr` inside a
 * 53:7 box keeps them square at any width, and the month strip uses the same
 * track template so a label always lands on the column that starts its month.
 */
const COLUMNS: CSSProperties = {
  display: 'grid',
  gridTemplateColumns: `repeat(${WEEKS}, minmax(0, 1fr))`,
  gap: GAP,
};

const MONTH_STYLE: CSSProperties = { ...COLUMNS, gridTemplateRows: '1fr' };

const GRID_STYLE: CSSProperties = {
  ...COLUMNS,
  gridTemplateRows: 'repeat(7, minmax(0, 1fr))',
  aspectRatio: `${WEEKS} / 7`,
  gridAutoFlow: 'column',
};

const WEEKDAY_STYLE: CSSProperties = {
  display: 'grid',
  gridTemplateRows: 'repeat(7, minmax(0, 1fr))',
  gap: GAP,
};

interface Cell {
  key: string;
  date: Date;
  tokens: number;
  requests: number;
  level: number;
  /** Days after today are drawn as empty, never as a day with zero usage. */
  future: boolean;
}

/**
 * The year at a glance: one cell per day, shaded by token volume.
 *
 * This is the view the bar chart cannot give — the bars answer "what did this
 * week look like", the calendar answers "am I using this every day, or in a
 * burst every other Tuesday". Cells are shaded by *quantile* rather than a
 * linear scale, because session usage is spiky: one 90M day would flatten every
 * other day into the lightest shade and the daily pattern would disappear.
 *
 * A day with no usage is a faint cell, never a missing one — a gap would
 * conflate "nothing happened" with "the app was not running".
 */
export function UsageCalendar({ provider, providerLabel }: UsageCalendarProps) {
  const { series, error, loading } = useCalendar(provider);
  const [hoveredKey, setHoveredKey] = useState<string | null>(null);

  const { cells, months, ceiling, activeDays, totalTokens } = useMemo(
    () => buildCalendar(series?.points ?? []),
    [series],
  );

  const hovered = hoveredKey ? cells.find((cell) => cell.key === hoveredKey) ?? null : null;
  const peak = useMemo(
    () =>
      cells.reduce<Cell | null>(
        (best, cell) => (cell.tokens > (best?.tokens ?? 0) ? cell : best),
        null,
      ),
    [cells],
  );

  return (
    <section className="panel">
      <div className="panel-head">
        <span className="label">Calendar</span>
        <span className="mono text-[10px] text-bone-3">
          {hovered ? (
            <span className="text-bone">
              <span className="text-bone-3">{formatDayLong(hovered.date)}</span>{' '}
              {hovered.tokens > 0 ? (
                <>
                  {formatTokens(hovered.tokens)} tok
                  <span className="text-bone-3"> · {hovered.requests} req</span>
                </>
              ) : (
                <span className="text-bone-3">no usage</span>
              )}
            </span>
          ) : loading ? (
            'loading'
          ) : (
            <span
              title={
                peak
                  ? `Busiest day: ${peak.tokens.toLocaleString('en-US')} tokens on ${formatDayLong(peak.date)}`
                  : undefined
              }
            >
              {activeDays} active {activeDays === 1 ? 'day' : 'days'} ·{' '}
              {formatTokens(totalTokens)} tokens ·{' '}
              {providerLabel ? `${providerLabel} · ` : ''}last 12 months
            </span>
          )}
        </span>
      </div>

      {error && (
        <p className="px-4 py-3 text-[11px] text-alert">Calendar unavailable: {error}</p>
      )}

      {!error && (
        <div className="overflow-x-auto px-4 pb-3 pt-2.5">
          <div className="inline-block">
            {/* Month ticks sit above the column where each month starts. */}
            <div className="ml-[26px]" style={MONTH_STYLE}>
              {months.map((month) => (
                <span
                  key={`${month.label}-${month.column}`}
                  className="label whitespace-nowrap"
                  style={{ gridColumnStart: month.column + 1, gridRow: 1, lineHeight: '11px' }}
                >
                  {month.label}
                </span>
              ))}
            </div>

            <div className="mt-1 flex items-stretch" style={{ gap: 6 }}>
              <div className="w-[20px] shrink-0" style={WEEKDAY_STYLE} aria-hidden>
                {['Mon', '', 'Wed', '', 'Fri', '', ''].map((day, index) => (
                  <span
                    key={index}
                    className="label flex items-center"
                    style={{ fontSize: 8 }}
                  >
                    {day.slice(0, 2)}
                  </span>
                ))}
              </div>

              <div className="min-w-0 flex-1" style={GRID_STYLE}>
                {cells.map((cell) =>
                  cell.future ? (
                    <span key={cell.key} aria-hidden />
                  ) : (
                    <button
                      key={cell.key}
                      type="button"
                      onMouseEnter={() => setHoveredKey(cell.key)}
                      onMouseLeave={() =>
                        setHoveredKey((current) => (current === cell.key ? null : current))
                      }
                      onFocus={() => setHoveredKey(cell.key)}
                      onBlur={() =>
                        setHoveredKey((current) => (current === cell.key ? null : current))
                      }
                      title={`${formatDayLong(cell.date)} — ${
                        cell.tokens > 0
                          ? `${cell.tokens.toLocaleString('en-US')} tokens in ${cell.requests} request(s)`
                          : 'no usage recorded'
                      }`}
                      aria-label={`${formatDayLong(cell.date)}: ${
                        cell.tokens > 0
                          ? `${cell.tokens.toLocaleString('en-US')} tokens in ${cell.requests} requests`
                          : 'no usage'
                      }`}
                      className={`rounded-[1.5px] transition-shadow ${
                        hoveredKey === cell.key ? 'shadow-[0_0_0_1px_var(--color-bone)]' : ''
                      } ${cell.tokens > 0 ? 'cursor-crosshair' : 'cursor-default'}`}
                      style={{
                        background:
                          cell.level === 0
                            ? 'color-mix(in srgb, var(--color-bone) 7%, transparent)'
                            : levelColor(cell.level),
                      }}
                    />
                  ),
                )}
              </div>
            </div>

            <div className="mt-2.5 flex items-center pl-[26px]" style={{ gap: 4 }}>
              <span className="label mr-1">less</span>
              {[0, 1, 2, 3, 4].map((level) => (
                <span
                  key={level}
                  className="rounded-[1.5px]"
                  style={{
                    width: 9,
                    height: 9,
                    background:
                      level === 0
                        ? 'color-mix(in srgb, var(--color-bone) 7%, transparent)'
                        : levelColor(level),
                  }}
                />
              ))}
              <span className="label ml-1">more</span>
              {ceiling > 0 && (
                <span className="mono ml-auto text-[9px] text-bone-3">
                  busiest day {formatTokens(ceiling)}
                </span>
              )}
            </div>
          </div>
        </div>
      )}
    </section>
  );
}

function levelColor(level: number): string {
  const percent = LEVEL_PERCENT[Math.min(level, LEVELS)] ?? 100;
  return `color-mix(in srgb, var(--color-needle) ${percent}%, transparent)`;
}

interface CalendarData {
  cells: Cell[];
  months: { column: number; label: string }[];
  ceiling: number;
  activeDays: number;
  totalTokens: number;
}

function buildCalendar(points: { key: string; tokens: number; requests: number }[]): CalendarData {
  const byDay = new Map<string, { tokens: number; requests: number }>();
  for (const point of points) {
    const current = byDay.get(point.key) ?? { tokens: 0, requests: 0 };
    byDay.set(point.key, {
      tokens: current.tokens + point.tokens,
      requests: current.requests + point.requests,
    });
  }

  const today = startOfDay(new Date());
  const end = addDays(today, 6 - weekdayIndex(today)); // Sunday of the current week
  const start = addDays(end, -(DAYS - 1));

  const raw: Cell[] = [];
  for (let index = 0; index < DAYS; index++) {
    const date = addDays(start, index);
    const key = dayKey(date);
    const entry = byDay.get(key);
    raw.push({
      key,
      date,
      tokens: entry?.tokens ?? 0,
      requests: entry?.requests ?? 0,
      level: 0,
      future: date.getTime() > today.getTime(),
    });
  }

  const totals = raw
    .filter((cell) => cell.tokens > 0)
    .map((cell) => cell.tokens)
    .sort((a, b) => a - b);
  const thresholds = [0.25, 0.5, 0.75, 1].map(
    (quantile) =>
      totals[Math.min(totals.length - 1, Math.floor(totals.length * quantile))] ?? 0,
  );

  const cells = raw.map((cell) => ({ ...cell, level: levelFor(cell.tokens, thresholds) }));

  const months: { column: number; label: string }[] = [];
  let lastColumn = -2;
  for (let column = 0; column < WEEKS; column++) {
    const date = cells[column * 7]?.date;
    if (!date || date.getDate() > 7) continue;
    if (column - lastColumn < 2) continue;
    months.push({
      column,
      label: date.toLocaleDateString('en-GB', { month: 'short' }),
    });
    lastColumn = column;
  }

  return {
    cells,
    months,
    ceiling: totals.length > 0 ? totals[totals.length - 1] : 0,
    activeDays: cells.filter((cell) => cell.tokens > 0).length,
    totalTokens: cells.reduce((sum, cell) => sum + cell.tokens, 0),
  };
}

function levelFor(tokens: number, thresholds: number[]): number {
  if (tokens <= 0) return 0;
  for (let level = 1; level <= LEVELS; level++) {
    if (tokens <= (thresholds[level - 1] ?? 0)) return level;
  }
  return LEVELS;
}

function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function addDays(date: Date, days: number): Date {
  const copy = new Date(date.getTime());
  copy.setDate(copy.getDate() + days);
  return copy;
}

/** Monday = 0 … Sunday = 6, unlike `Date.getDay()`. */
function weekdayIndex(date: Date): number {
  return (date.getDay() + 6) % 7;
}

function pad2(n: number): string {
  return String(n).padStart(2, '0');
}

function dayKey(date: Date): string {
  return `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(date.getDate())}`;
}

function formatDayLong(date: Date): string {
  return date.toLocaleDateString('en-GB', {
    weekday: 'short',
    day: '2-digit',
    month: 'short',
    year: 'numeric',
  });
}
