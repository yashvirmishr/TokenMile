import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { UsageSeries } from '../types';

/** A calendar year, expressed as whole weeks so the grid always ends today. */
export const CALENDAR_DAYS = 371;

/**
 * Daily totals for the calendar.
 *
 * Fetched separately from the dashboard on purpose: the calendar is always the
 * last year at day resolution, while the dashboard series follows the selected
 * range. This keeps a range switch from re-reading a year of daily buckets it
 * cannot use, and keeps the calendar stable while the rest of the page changes.
 */
export function useCalendar(provider: string | null, enabled = true) {
  const [series, setSeries] = useState<UsageSeries | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    if (!enabled) return;
    try {
      const result = await invoke<UsageSeries>('get_calendar', {
        days: CALENDAR_DAYS,
        provider,
      });
      setSeries(result);
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [provider, enabled]);

  useEffect(() => {
    load();
  }, [load]);

  // Follow the same refresh signal the dashboard uses, so the calendar gains a
  // cell the moment a background pass imports today's usage.
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let active = true;

    listen('sync://complete', () => {
      load();
    })
      .then((fn) => {
        if (active) unlisten = fn;
        else fn();
      })
      .catch(() => {
        // The dashboard already surfaces a broken event channel; a calendar that
        // silently stops updating is not worth a second error banner.
      });

    return () => {
      active = false;
      unlisten?.();
    };
  }, [load]);

  return { series, error, loading, reload: load };
}
