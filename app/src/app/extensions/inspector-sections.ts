import { inject, Injectable } from '@angular/core';
import { RecordView } from '../core/api.types';
import { flattenContributions, IMT_INSPECTOR_SECTIONS, ImtInspectorSection } from './tokens';

/** Plugin sections for the inspector, ordered, filtered per record. */
@Injectable({ providedIn: 'root' })
export class InspectorSections {
  private readonly sections = flattenContributions<ImtInspectorSection>(inject(IMT_INSPECTOR_SECTIONS, { optional: true })).sort(
    (a, b) => a.order - b.order,
  );

  forRecord(record: RecordView): ImtInspectorSection[] {
    return this.sections.filter((s) => {
      try {
        return s.when ? s.when(record) : true;
      } catch {
        return false;
      }
    });
  }
}
