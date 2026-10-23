import { DebriefView } from '../../extensions/tokens';

/** The dashboard view: the profile's dashboards as a grid of tiles. */
export const DASHBOARD_VIEWS: DebriefView[] = [
  {
    id: 'dashboards',
    title: $localize`:@@share.dashboardViews.title:Dashboards`,
    category: 'Sharing',
    icon: 'chart',
    defaultPosition: 'center', layout: { area: 'document', min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 55 },
    component: () => import('./dashboard-view').then((m) => m.DashboardView),
  },
];
