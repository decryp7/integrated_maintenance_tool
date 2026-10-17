import { inject, Injectable } from '@angular/core';
import { formatDateTime, formatDuration, tzLabel } from '../core/format';
import { ImtEvents } from '../extensions/events';
import { TimeRange } from './time-cursor';

/** Formats instants in the zone the user chose for the app (read only). */
@Injectable({ providedIn: 'root' })
export class AnalysisTime {
  private readonly events = inject(ImtEvents);

  format(ts: number | null | undefined, ms = false): string {
    return ts === null || ts === undefined ? '—' : formatDateTime(ts, this.events.timezone(), ms);
  }

  range(r: TimeRange | null): string {
    return r ? `${this.format(r.from)} → ${this.format(r.to)} ${tzLabel(this.events.timezone())} (${formatDuration(r.to - r.from)})` : '—';
  }
}
