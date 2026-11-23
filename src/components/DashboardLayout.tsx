import type { ReactNode } from 'react';
import {
  sectionSpan,
  type DashboardSectionId,
  type ThemeLayout,
} from '../theme/schema';

interface DashboardLayoutProps {
  layout: ThemeLayout;
  /** One rendered section per id. A full UI fork can replace any of them. */
  sections: Record<DashboardSectionId, ReactNode>;
}

/**
 * Render dashboard sections in the order and visibility a theme pack declares.
 *
 * This is the seam that makes community UIs possible without forking the data
 * layer: the layout owns arrangement, each section owns its own component, and
 * `useDashboard()` owns the backend contract. A fork can swap a section, hide
 * one from its theme JSON, or render the map itself.
 */
export function DashboardLayout({ layout, sections }: DashboardLayoutProps) {
  const visible = layout.order.filter((id) => !layout.hidden.includes(id));

  return (
    <div className="grid grid-cols-1 items-start gap-3 lg:grid-cols-12">
      {visible.map((id, index) => {
        const span = layout.spans[id] ?? sectionSpan(id);
        return (
          <div
            key={id}
            className={`reveal min-w-0 ${SPAN_CLASSES[span] ?? 'lg:col-span-12'}`}
            style={{ ['--reveal-delay' as string]: `${Math.min(index * 40, 280)}ms` }}
          >
            {sections[id]}
          </div>
        );
      })}
    </div>
  );
}

/** Static class names keep Tailwind's scanner able to see every variant. */
const SPAN_CLASSES: Record<number, string> = {
  1: 'lg:col-span-1',
  2: 'lg:col-span-2',
  3: 'lg:col-span-3',
  4: 'lg:col-span-4',
  5: 'lg:col-span-5',
  6: 'lg:col-span-6',
  7: 'lg:col-span-7',
  8: 'lg:col-span-8',
  9: 'lg:col-span-9',
  10: 'lg:col-span-10',
  11: 'lg:col-span-11',
  12: 'lg:col-span-12',
};
