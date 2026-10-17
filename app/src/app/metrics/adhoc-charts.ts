import { Injectable, signal } from '@angular/core';
import { ChartDef } from '../core/api.types';

/** Charts the user built in the Metrics panel (or from a field); they live for the session, not in the profile. */
@Injectable({ providedIn: 'root' })
export class AdhocCharts {
  private seq = 0;
  private readonly list = signal<readonly ChartDef[]>([]);
  readonly charts = this.list.asReadonly();

  add(def: Omit<ChartDef, 'id'>): ChartDef {
    const chart = { ...def, id: `adhoc-${++this.seq}` };
    this.list.update((l) => [...l, chart]);
    return chart;
  }

  /** `avg(field)` over time, split by the given fields. Reuses an existing chart for the same field. */
  addForField(field: string, splitBy: string[]): ChartDef {
    const y = `avg(${field})`;
    const existing = this.list().find((c) => c.y === y && c.splitBy.join() === splitBy.join());
    return existing ?? this.add({ title: field, y, bucket: 'auto', splitBy, type: 'line', unit: '' });
  }

  remove(id: string): void {
    this.list.update((l) => l.filter((c) => c.id !== id));
  }
}
