import { inject } from '@angular/core';
import { RecordMenuContribution } from '../../context/record-actions';
import { ReportActions } from './report-actions';

/** "Add to report" in the record context menu: the record, or every record of the grid's multi-selection. */
export function reportMenuItems(): RecordMenuContribution {
  const actions = inject(ReportActions);
  return (record, target) => {
    const ids = target.records?.length ? target.records.map((r) => r.id) : [record.id];
    return [{ label: ids.length > 1 ? $localize`:@@share.reportMenu.addMany:Add ${ids.length}:count: events to report` : $localize`:@@share.reportMenu.add:Add to report`, run: () => void actions.addEvents(ids) }];
  };
}
