import { viewCategoryMenu } from '../extensions/activity-items';
import { DebriefActivityItem } from '../extensions/tokens';

/**
 * The Tools fly-out: the tool views this platform lists (the catalog decides through each view's `enabled`), the
 * collection schedules and runs, then the diagnostics collector. Call it in an injection context.
 */
export function toolsActivityItems(): DebriefActivityItem[] {
  return [
    {
      id: 'tools',
      label: $localize`:@@tools.activity.tools:Tools`,
      icon: 'tools',
      order: 800,
      run: { menu: viewCategoryMenu('Tools', { first: ['process-monitor', 'event-log-follower', 'perfmon'], commands: ['tools.collect'] }) },
    },
  ];
}
