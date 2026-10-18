import { ImtActivityItem } from '../extensions/tokens';
import { LiveTail } from '../workspace/live-tail';
import { LiveFollow } from './live-follow.store';

/** The Live tail toggle in the activity bar: pressed while live tail is on, with a pulsing dot while following. */
export function liveActivityItems(live: LiveTail, follow: InstanceType<typeof LiveFollow>): ImtActivityItem[] {
  return [
    {
      id: 'live-tail',
      label: $localize`:@@live.activity.toggle:Toggle live tail`,
      icon: 'live',
      order: 600,
      toggle: true,
      active: () => live.enabled(),
      pulse: () => follow.active(),
      disabled: () => (live.available() ? null : $localize`:@@live.activity.unavailable:Open files first to follow them live`),
      run: { command: 'live.toggle' },
    },
  ];
}
