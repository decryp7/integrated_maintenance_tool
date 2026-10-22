import { DebriefActivityItem } from '../extensions/tokens';
import { LiveTail } from '../workspace/live-tail';
import { LiveFollow } from './live-follow.store';

/** The Live tail toggle in the activity bar: pressed while live tail is on, with a pulsing dot while following; shown while there are files to follow. */
export function liveActivityItems(live: LiveTail, follow: InstanceType<typeof LiveFollow>): DebriefActivityItem[] {
  return [
    {
      id: 'live-tail',
      label: $localize`:@@live.activity.toggle:Toggle live tail`,
      icon: 'live',
      order: 600,
      toggle: true,
      active: () => live.enabled(),
      pulse: () => follow.active(),
      // Only with files open (File ▸ Live tail and the View menu stay available).
      when: () => live.available(),
      run: { command: 'live.toggle' },
    },
  ];
}
