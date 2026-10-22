import { DebriefView } from '../extensions/tokens';
import { DASHBOARD_VIEWS } from './dashboard/dashboard.views';
import { REPORT_VIEWS } from './report/report.views';

/** The dockable views of the "explain and share" features. */
export const SHARE_VIEWS: DebriefView[] = [...REPORT_VIEWS, ...DASHBOARD_VIEWS];
