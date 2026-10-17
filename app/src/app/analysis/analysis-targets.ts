import { computed, inject, Injectable, signal } from '@angular/core';
import { ImtEvents } from '../extensions/events';
import { QlQuery } from '../core/api.types';
import { TimeCursor, TimeRange } from './time-cursor';

/** What the analysis views were asked to look at: the moment, the two compare windows, a field. */
@Injectable({ providedIn: 'root' })
export class AnalysisTargets {
  private readonly events = inject(ImtEvents);
  private readonly cursor = inject(TimeCursor);

  private readonly momentSignal = signal<TimeRange | null>(null);
  private readonly aSignal = signal<TimeRange | null>(null);
  private readonly bSignal = signal<TimeRange | null>(null);
  private readonly fieldSignal = signal<string | null>(null);

  readonly moment = this.momentSignal.asReadonly();
  readonly windowA = this.aSignal.asReadonly();
  readonly windowB = this.bSignal.asReadonly();
  readonly field = this.fieldSignal.asReadonly();

  /** The query as the engine receives it (includes `ql` when the workspace has one). */
  readonly query = computed(() => this.events.query() as QlQuery);

  /** Selected range, else the query's range, else the whole workspace; null while nothing is loaded. */
  currentRange(): TimeRange | null {
    const sel = this.cursor.selection();
    if (sel && sel.to > sel.from) return sel;
    const q = this.events.query();
    const w = this.events.workspace();
    const from = q.from ?? w?.from ?? null;
    const to = q.to ?? w?.to ?? null;
    return from !== null && to !== null && to >= from ? { from, to } : null;
  }

  setMoment(range: TimeRange | null): void {
    this.momentSignal.set(range);
  }
  setWindows(a: TimeRange | null, b: TimeRange | null): void {
    this.aSignal.set(a);
    this.bSignal.set(b);
  }
  setWindowA(range: TimeRange | null): void {
    this.aSignal.set(range);
  }
  setWindowB(range: TimeRange | null): void {
    this.bSignal.set(range);
  }
  setField(field: string | null): void {
    this.fieldSignal.set(field);
  }
}

/** The query restricted to a window (the window replaces from/to). */
export function windowQuery(base: QlQuery, range: TimeRange | null): QlQuery {
  return range ? { ...base, from: range.from, to: range.to } : base;
}
