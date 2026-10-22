import { inject, Injectable } from '@angular/core';
import { QlQuery } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { DebriefEvents } from '../extensions/events';
import { andQl } from './ql-text';
import { TimeRange } from './time-cursor';

/** What views do to the rest of the app, always through the narrow `CommandContext`. */
@Injectable({ providedIn: 'root' })
export class AnalysisActions {
  private readonly ctx = inject(COMMAND_CONTEXT);
  private readonly events = inject(DebriefEvents);
  private readonly analysis = inject(AnalysisBackend);

  /** Sets the workspace query's `ql` (filters grid, timeline and charts). */
  setQl(ql: string): void {
    this.ctx.setQuery({ ql } as Partial<QlQuery>);
  }

  /** ANDs `clause` with the workspace's current `ql` (parenthesised where needed). */
  narrowQl(clause: string): void {
    this.setQl(andQl(this.events.query().ql ?? '', clause));
  }

  zoomTo(range: TimeRange): void {
    this.ctx.setQuery({ from: range.from, to: range.to });
  }

  /** Adds an equality filter pill for a field value. */
  filterField(field: string, value: string): void {
    this.ctx.addFilter({ field, op: 'eq', value });
  }

  showView(id: string): void {
    this.ctx.showView(id);
  }

  fail(message: string): void {
    this.ctx.notifyError(message);
  }

  /** Reveals an event in the grid (loads its page, scrolls to it and selects it). */
  jumpTo(id: number): Promise<void> {
    return this.ctx.jumpTo(id);
  }

  /** Zooms to `range` and reveals the first sample event of an anomaly in the zoomed grid. */
  async revealSample(range: TimeRange, id: number | undefined): Promise<void> {
    this.zoomTo(range);
    if (id !== undefined) await this.jumpTo(id);
  }

  /**
   * Filters the workspace to one request id. `idField` may list alternatives (`A|B`); the engine's
   * `correlate` tells which of them the id belongs to. Returns the number of matching events.
   */
  async filterById(idField: string, id: string): Promise<number> {
    for (const field of idField.split('|').map((f) => f.trim()).filter(Boolean)) {
      const { result: ids } = await this.analysis.correlate(field, id);
      if (ids.length) {
        this.ctx.addFilter({ field, op: 'eq', value: id });
        return ids.length;
      }
    }
    this.ctx.notifyError($localize`:@@analysis.actions.noEventsForId:No events carry the id ${id}:id:.`);
    return 0;
  }
}
