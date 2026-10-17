import { computed, inject, Injectable, signal } from '@angular/core';
import { QlError, QlQuery, QlResult, Completion } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { ImtEvents } from '../extensions/events';
import { AnalysisActions } from './analysis-actions';
import { errorText } from './async-resource';
import { QueryHistory } from './history-store';

export const CHECK_DELAY_MS = 150;
const FOCUS_REQUEST_TTL_MS = 5000;

export type TableResult = Extract<QlResult, { kind: 'table' }>;

/**
 * State of the query bar: text, live syntax check, completions, history and the last table result.
 * Everything about the language (errors, completions, what a result is) comes from the engine.
 */
@Injectable({ providedIn: 'root' })
export class QuerySession {
  private readonly backend = inject(AnalysisBackend);
  private readonly events = inject(ImtEvents);
  private readonly actions = inject(AnalysisActions);
  readonly history = inject(QueryHistory);

  readonly text = signal('');
  /** Syntax error of the current text, from `check_query`. */
  readonly error = signal<QlError | null>(null);
  /** Error from the last Run (syntax or execution). */
  readonly runError = signal<string | null>(null);
  readonly completions = signal<Completion[]>([]);
  readonly running = signal(false);
  readonly table = signal<TableResult | null>(null);
  /** Number of events matched by the last event-returning run. */
  readonly matched = signal<number | null>(null);

  /** `ql` currently applied to the workspace. */
  readonly applied = computed(() => (this.events.query() as QlQuery).ql ?? '');

  /** Bumped by the "focus query bar" command; the view focuses its input when it changes. */
  readonly focusNonce = signal(0);

  private focusRequestedAt = Number.NEGATIVE_INFINITY;
  private checkTimer: ReturnType<typeof setTimeout> | undefined;
  private seq = 0;

  /** Updates the text and schedules the syntax check; completions refresh for the cursor. */
  edit(text: string, cursorBytes: number): void {
    this.text.set(text);
    this.runError.set(null);
    clearTimeout(this.checkTimer);
    const mine = ++this.seq;
    this.checkTimer = setTimeout(() => void this.refresh(text, cursorBytes, mine), CHECK_DELAY_MS);
  }

  /** Completions at a cursor (UTF-8 byte offset) without touching the text. */
  async complete(cursorBytes: number): Promise<void> {
    const mine = ++this.seq;
    await this.refresh(this.text(), cursorBytes, mine);
  }

  private async refresh(text: string, cursorBytes: number, mine: number): Promise<void> {
    try {
      const [error, completions] = await Promise.all([text.trim() ? this.backend.checkQuery(text) : Promise.resolve(null), this.backend.completeQuery(text, cursorBytes)]);
      if (mine !== this.seq) return;
      this.error.set(error);
      this.completions.set(completions);
    } catch {
      if (mine === this.seq) this.completions.set([]);
    }
  }

  dismissCompletions(): void {
    this.completions.set([]);
  }

  /**
   * Runs the text. An events result becomes the workspace query's `ql`; a table result (`stats`)
   * is kept for the results view. Returns the kind of result, or null on error.
   */
  async run(): Promise<'events' | 'table' | null> {
    const text = this.text();
    if (!text.trim()) {
      this.actions.setQl('');
      this.table.set(null);
      this.matched.set(null);
      this.runError.set(null);
      return 'events';
    }
    this.running.set(true);
    this.runError.set(null);
    try {
      const outcome = await this.backend.runQuery({ ...(this.events.query() as QlQuery), ql: text });
      if ('error' in outcome) {
        this.error.set(outcome.error);
        this.runError.set(outcome.error.message);
        return null;
      }
      this.error.set(null);
      this.history.add(text);
      if (outcome.result.kind === 'table') {
        this.table.set(outcome.result);
        this.matched.set(null);
        this.actions.showView('query-results');
        return 'table';
      }
      this.table.set(null);
      this.matched.set(outcome.result.ids.length);
      this.actions.setQl(text);
      return 'events';
    } catch (e) {
      this.runError.set(errorText(e));
      return null;
    } finally {
      this.running.set(false);
    }
  }

  /** Removes the applied `ql` from the workspace query. */
  clearApplied(): void {
    this.actions.setQl('');
    this.matched.set(null);
  }

  requestFocus(): void {
    this.focusRequestedAt = Date.now();
    this.focusNonce.update((n) => n + 1);
  }

  /** True once per `requestFocus()` and only shortly after it: the view may be created after the command ran. */
  takeFocusRequest(): boolean {
    const wanted = Date.now() - this.focusRequestedAt < FOCUS_REQUEST_TTL_MS;
    this.focusRequestedAt = Number.NEGATIVE_INFINITY;
    return wanted;
  }

  load(text: string): void {
    this.edit(text, new TextEncoder().encode(text).length);
  }
}
