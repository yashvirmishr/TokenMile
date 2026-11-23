import type { ReactNode } from 'react';
import type {
  ActivityEntry,
  ModelUsage,
  ProviderStatus,
  ProviderUsage,
  TimeRange,
  UsageSeries,
  UsageSummary,
} from '../types';
import type { DashboardSectionId } from '../theme/schema';
import { ActivityFeed } from './ActivityFeed';
import { ModelUsageTable } from './ModelUsageTable';
import { ProviderStatusPanel } from './ProviderStatus';
import { ProviderTable } from './ProviderTable';
import { UsageCalendar } from './UsageCalendar';
import { UsageChart } from './UsageChart';
import { UsageHeadline } from './UsageHeadline';
import { ValueDisplay } from './ValueDisplay';

export interface DefaultSectionsInput {
  summary: UsageSummary | null;
  series: UsageSeries | null;
  providers: ProviderStatus[];
  providerRows: ProviderUsage[];
  models: ModelUsage[];
  activity: ActivityEntry[];
  range: TimeRange;
  filterLabel: string;
  /** Selected provider id, or null for every provider. */
  providerFilter: string | null;
  /** The provider filter row, kept as a node so a fork can replace it. */
  filters: ReactNode;
}

/**
 * The stock dashboard as a section map.
 *
 * This exists so a community UI does not have to copy the whole of `App.tsx` to
 * change one panel:
 *
 * ```tsx
 * const sections = {
 *   ...createDefaultSections(input),
 *   headline: <MyHeadline />,
 * };
 * ```
 *
 * A theme pack can then order or hide any of these ids. If a fork needs a new
 * section, add its id to `DASHBOARD_SECTIONS` and add an entry here or in its
 * own map.
 */
export function createDefaultSections(
  input: DefaultSectionsInput,
): Record<DashboardSectionId, ReactNode> {
  const providerLabel = input.providerFilter
    ? input.providers.find((provider) => provider.id === input.providerFilter)
        ?.short_label ?? input.providerFilter
    : null;

  return {
    headline: (
      <UsageHeadline
        summary={input.summary}
        range={input.range}
        modelCount={input.models.length}
        providerCount={input.providerRows.length}
      />
    ),
    chart: (
      <UsageChart
        series={input.series}
        range={input.range}
        providers={input.providers}
        filterLabel={input.filterLabel}
      />
    ),
    // The calendar always spans a year of days, so it reads the provider filter
    // but deliberately ignores the time range.
    calendar: (
      <UsageCalendar
        provider={input.providerFilter}
        providerLabel={providerLabel}
      />
    ),
    value: <ValueDisplay summary={input.summary} />,
    filters: input.filters,
    providers: (
      <ProviderTable
        providers={input.providerRows}
        status={input.providers}
        rangeLabel={input.filterLabel}
      />
    ),
    activity: <ActivityFeed entries={input.activity} status={input.providers} />,
    models: <ModelUsageTable models={input.models} status={input.providers} />,
    status: <ProviderStatusPanel providers={input.providers} />,
  };
}
