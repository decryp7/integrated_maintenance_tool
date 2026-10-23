import { DebriefView } from '../../extensions/tokens';

/** The report builder view. */
export const REPORT_VIEWS: DebriefView[] = [
  { id: 'report', category: 'Sharing', title: $localize`:@@share.reportViews.title:Report`, icon: 'file', defaultPosition: 'right', area: 'document', component: () => import('./report-view').then((m) => m.ReportView) },
];
