import { viewCategoryMenu, whenDataOpen } from '../extensions/activity-items';
import { DebriefActivityItem } from '../extensions/tokens';
import { ANALYSIS_MENU_ORDER } from './analysis-menu-order';

/** The Analysis fly-out: every offered view of the category, from the view registry. Call it in an injection context. */
export function analysisActivityItems(): DebriefActivityItem[] {
  return [{ id: 'analysis', label: $localize`:@@analysis.activity.analysis:Analysis`, icon: 'analysis', order: 400, when: whenDataOpen, run: { menu: viewCategoryMenu('Analysis', { first: ANALYSIS_MENU_ORDER }) } }];
}
