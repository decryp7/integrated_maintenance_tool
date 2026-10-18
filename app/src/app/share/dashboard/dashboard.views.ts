import { ImtView } from '../../extensions/tokens';

/** The dashboard view: the profile's dashboards as a grid of tiles. */
export const DASHBOARD_VIEWS: ImtView[] = [
  {
    id: 'dashboards',
    title: $localize`:@@share.dashboardViews.title:Dashboards`,
    category: 'Sharing',
    icon: 'chart',
    defaultPosition: 'center',
    component: () => import('./dashboard-view').then((m) => m.DashboardView),
  },
];
