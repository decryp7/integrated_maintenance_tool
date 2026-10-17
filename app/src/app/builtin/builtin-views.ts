import { ImtView } from '../extensions/tokens';

/** The panels IMT ships with. They use the same extension point as plugin views. */
export const BUILTIN_VIEWS: ImtView[] = [
  { id: 'timeline', category: 'core', title: 'Timeline', icon: 'timeline', defaultPosition: 'center', component: () => import('../timeline/timeline-panel').then((m) => m.TimelinePanel) },
  { id: 'events', category: 'core', title: 'Events', icon: 'columns', defaultPosition: 'bottom', component: () => import('../grid/events-grid').then((m) => m.EventsGrid) },
  { id: 'inspector', category: 'core', title: 'Inspector', icon: 'panel', defaultPosition: 'right', component: () => import('../inspector/inspector').then((m) => m.Inspector) },
  { id: 'metrics', category: 'core', title: 'Metrics', icon: 'metrics', defaultPosition: 'center', component: () => import('../metrics/metrics-view').then((m) => m.MetricsView) },
  { id: 'profile', category: 'core', title: 'Profile', icon: 'profile', defaultPosition: 'center', component: () => import('../profile/profile-editor').then((m) => m.ProfileEditor) },
  { id: 'plugins', category: 'Plugins', title: 'Plugins', icon: 'puzzle', defaultPosition: 'center', component: () => import('../plugins-panel/plugins-view').then((m) => m.PluginsView) },
];
