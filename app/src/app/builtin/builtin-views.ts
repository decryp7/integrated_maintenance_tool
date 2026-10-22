import { ImtView } from '../extensions/tokens';

/** The panels IMT ships with. They use the same extension point as plugin views. */
export const BUILTIN_VIEWS: ImtView[] = [
  { id: 'timeline', category: 'core', title: $localize`:@@builtin.view.timeline:Timeline`, icon: 'timeline', defaultPosition: 'center', instances: true, component: () => import('../timeline/timeline-panel').then((m) => m.TimelinePanel) },
  { id: 'events', category: 'core', title: $localize`:@@builtin.view.events:Events`, icon: 'columns', defaultPosition: 'bottom', area: 'document', component: () => import('../grid/events-grid').then((m) => m.EventsGrid) },
  { id: 'inspector', category: 'core', title: $localize`:@@builtin.view.inspector:Inspector`, icon: 'panel', defaultPosition: 'right', component: () => import('../inspector/inspector').then((m) => m.Inspector) },
  { id: 'metrics', category: 'core', title: $localize`:@@builtin.view.metrics:Metrics`, icon: 'metrics', defaultPosition: 'center', component: () => import('../metrics/metrics-view').then((m) => m.MetricsView) },
  { id: 'profile', category: 'core', title: $localize`:@@builtin.view.profile:Profile`, icon: 'profile', defaultPosition: 'center', minSize: { width: 360, height: 240 }, component: () => import('../profile/profile-editor').then((m) => m.ProfileEditor) },
  { id: 'plugins', category: 'Plugins', title: $localize`:@@builtin.view.plugins:Plugins`, icon: 'puzzle', defaultPosition: 'center', area: 'status', component: () => import('../plugins-panel/plugins-view').then((m) => m.PluginsView) },
];
