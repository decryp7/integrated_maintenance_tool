import { DebriefView } from '../../extensions/tokens';

/** The report builder view. */
export const REPORT_VIEWS: DebriefView[] = [
  { id: 'report', category: 'Sharing', title: $localize`:@@share.reportViews.title:Report`, icon: 'file', defaultPosition: 'right', layout: { area: 'document', min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 55 }, component: () => import('./report-view').then((m) => m.ReportView) },
];
