import { WorkspaceStore } from '../core/workspace.store';
import { ImtActivityItem } from '../extensions/tokens';
import { whenDataOpen } from '../extensions/activity-items';

/**
 * The built-in activity-bar buttons: the Explorer and Search, the core views (Timeline, Metrics, Profile editor) and
 * Settings at the bottom. Search, Timeline and Metrics show only while data is open (they stay in the View menu). Features add theirs (Analysis, Live tail, AI, Tools) the same way plugins do.
 */
export function builtinActivityItems(store: InstanceType<typeof WorkspaceStore>): ImtActivityItem[] {
  return [
    { id: 'explorer', label: $localize`:@@shell.activityBar.explorer:Explorer`, icon: 'files', order: 100, toggle: true, active: () => store.sidebarOpen(), run: { command: 'view.toggleSidebar' } },
    { id: 'search', label: $localize`:@@shell.activityBar.search:Search`, icon: 'search', order: 200, keybinding: '/', when: whenDataOpen, run: { command: 'analyze.search' } },
    { id: 'timeline', label: $localize`:@@shell.activityBar.timeline:Timeline`, icon: 'timeline', order: 300, when: whenDataOpen, run: { view: 'timeline' } },
    { id: 'metrics', label: $localize`:@@shell.activityBar.metrics:Metrics`, icon: 'metrics', order: 500, when: whenDataOpen, run: { view: 'metrics' } },
    { id: 'profile', label: $localize`:@@shell.activityBar.profileEditor:Profile editor`, icon: 'profile', order: 900, run: { view: 'profile' } },
    { id: 'settings', label: $localize`:@@shell.activityBar.settings:Settings`, icon: 'settings', order: 100, section: 'bottom', run: { command: 'settings.open' } },
  ];
}
