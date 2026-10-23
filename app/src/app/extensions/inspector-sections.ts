import { inject, Injectable } from '@angular/core';
import { RecordView } from '../core/api.types';
import { flattenContributions, DEBRIEF_INSPECTOR_SECTIONS, DebriefInspectorSection } from './tokens';

/** Plugin sections for the inspector, ordered, filtered per record. */
@Injectable({ providedIn: 'root' })
export class InspectorSections {
  private readonly sections = flattenContributions<DebriefInspectorSection>(inject(DEBRIEF_INSPECTOR_SECTIONS, { optional: true })).sort(
    (a, b) => a.order - b.order,
  );

  forRecord(record: RecordView): DebriefInspectorSection[] {
    return this.sections.filter((s) => {
      try {
        return s.when ? s.when(record) : true;
      } catch {
        return false;
      }
    });
  }
}
