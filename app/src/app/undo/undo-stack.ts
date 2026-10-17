import { computed, Injectable, signal, WritableSignal } from '@angular/core';

/** One user edit that can be taken back: `do` (re)applies it, `undo` restores what was there before. */
export interface UndoableAction {
  /** Short, lower-case description for "Undo <label>", e.g. "filter change" or "marker rule edit". */
  label: string;
  do: () => void | Promise<void>;
  undo: () => void | Promise<void>;
  /**
   * Consecutive actions with the same key recorded within `MERGE_WINDOW_MS` are merged into one entry
   * (the first one's `undo`, the last one's `do`), e.g. typing in a search box.
   */
  mergeKey?: string;
  /** Entries of a group can be dropped together with `discard` (e.g. edits of a draft that was saved or cancelled). */
  group?: string;
}

/** Entries kept; the oldest are dropped beyond this. */
export const UNDO_LIMIT = 100;
/** See `UndoableAction.mergeKey`. */
export const MERGE_WINDOW_MS = 1500;

interface Entry extends UndoableAction {
  at: number;
}

/**
 * Command-pattern undo/redo for user edits (filters, marker and highlight rules, bookmarks, profile edits,
 * dashboards, reports). Features register what they did; undo and redo replay the closures in order, one
 * at a time. Engine-changing actions undo by re-applying the previous state through the normal validated
 * save path (see `ProfileUndo`). Failures are reported, never thrown, and leave the history where it was.
 */
@Injectable({ providedIn: 'root' })
export class UndoStack {
  private readonly past = signal<readonly Entry[]>([]);
  private readonly future = signal<readonly Entry[]>([]);
  private readonly running = signal(false);
  /** The last undo, redo or `execute` that failed (shown to the user by the caller); notifies on every failure. */
  readonly lastError = signal<string | null>(null, { equal: () => false });
  /** Clock, replaceable in tests. */
  now: () => number = () => Date.now();

  readonly canUndo = computed(() => this.past().length > 0 && !this.running());
  readonly canRedo = computed(() => this.future().length > 0 && !this.running());
  readonly undoLabel = computed(() => this.past().at(-1)?.label ?? null);
  readonly redoLabel = computed(() => this.future().at(-1)?.label ?? null);
  /** True while an undo or redo runs: observers must not record the changes it makes. */
  readonly applying = this.running.asReadonly();
  readonly size = computed(() => this.past().length);

  /** Records an edit that has already been made. Ignored while an undo or redo is being applied. */
  record(action: UndoableAction): void {
    if (this.running()) return;
    const at = this.now();
    const last = this.past().at(-1);
    if (action.mergeKey && last?.mergeKey === action.mergeKey && at - last.at <= MERGE_WINDOW_MS) {
      this.past.update((p) => [...p.slice(0, -1), { ...action, undo: last.undo, at }]);
    } else {
      this.past.update((p) => [...p, { ...action, at }].slice(-UNDO_LIMIT));
    }
    this.future.set([]);
  }

  /** Makes an edit (`do`) and records it. Resolves false (and records nothing) when `do` fails. */
  async execute(action: UndoableAction): Promise<boolean> {
    try {
      await action.do();
    } catch (e) {
      this.lastError.set(`${capitalise(action.label)} failed: ${message(e)}`);
      return false;
    }
    this.record(action);
    return true;
  }

  /** Takes back the latest edit. Resolves false when there is none, another one runs, or it failed. */
  undo(): Promise<boolean> {
    return this.step(this.past, this.future, 'undo');
  }

  /** Re-applies the latest undone edit. */
  redo(): Promise<boolean> {
    return this.step(this.future, this.past, 'do');
  }

  /** Forgets everything (another project or workspace replaces what the edits referred to). */
  clear(): void {
    this.past.set([]);
    this.future.set([]);
  }

  /** Forgets the entries of one group (their state no longer exists), keeping the others in order. */
  discard(group: string): void {
    this.past.update((s) => s.filter((e) => e.group !== group));
    this.future.update((s) => s.filter((e) => e.group !== group));
  }

  private async step(from: WritableSignal<readonly Entry[]>, to: WritableSignal<readonly Entry[]>, op: 'undo' | 'do'): Promise<boolean> {
    const entry = from().at(-1);
    if (!entry || this.running()) return false;
    this.running.set(true);
    try {
      await entry[op]();
      from.update((s) => s.slice(0, -1));
      to.update((s) => [...s, { ...entry, mergeKey: undefined }].slice(-UNDO_LIMIT));
      return true;
    } catch (e) {
      this.lastError.set(`Could not ${op === 'undo' ? 'undo' : 'redo'} ${entry.label}: ${message(e)}`);
      return false;
    } finally {
      this.running.set(false);
    }
  }
}

function capitalise(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
