import { ImtView } from '../../extensions/tokens';

/** The report builder view. */
export const REPORT_VIEWS: ImtView[] = [
  { id: 'report', category: 'Sharing', title: 'Report', icon: 'file', defaultPosition: 'right', component: () => import('./report-view').then((m) => m.ReportView) },
];
