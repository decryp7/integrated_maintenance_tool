import { inject, Injectable } from '@angular/core';
import { RecordView } from '../core/api.types';
import { valueToString } from '../core/format';
import { flattenContributions, IMT_FIELD_FORMATTERS, ImtFieldFormatter } from './tokens';

/** Picks the first formatter registered for a field. Pure, so it is easy to test. */
export function findFormatter(formatters: readonly ImtFieldFormatter[], field: string): ImtFieldFormatter | undefined {
  return formatters.find((f) => (typeof f.field === 'string' ? f.field === field : f.field.test(field)));
}

/** Formats field values for display, falling back to the plain string form. */
@Injectable({ providedIn: 'root' })
export class FieldFormatters {
  private readonly formatters = flattenContributions<ImtFieldFormatter>(inject(IMT_FIELD_FORMATTERS, { optional: true }));

  format(field: string, value: unknown, record: RecordView): string {
    const formatter = findFormatter(this.formatters, field);
    if (!formatter) return valueToString(value);
    try {
      return formatter.format(value, record);
    } catch {
      return valueToString(value);
    }
  }
}

/** Serilog EventId objects `{Id, Name}` as "10100 · Name". */
export const EVENT_ID_FORMATTER: ImtFieldFormatter = {
  field: /^EventId$/,
  format: (value) => {
    if (value && typeof value === 'object' && 'Id' in value) {
      const { Id, Name } = value as { Id: unknown; Name?: unknown };
      return Name ? `${String(Id)} · ${String(Name)}` : String(Id);
    }
    return valueToString(value);
  },
};
