import { computed, effect, inject, Injectable, Injector, signal, untracked } from '@angular/core';
import { AnalysisBackend } from '../core/analysis-backend';
import { SearchInputKind } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { QuerySession } from './query-session';

/** What the field last put into the workspace query itself (so that change is not mistaken for an outside one). */
interface Committed {
  text: string;
  ql: string;
}

/**
 * The filter bar's one search field (D2): plain words search the messages as you type; text with query structure is
 * a query-language query, checked as you type and run with Enter. The engine decides which (`classify_search`); the
 * text is the query session's, so history, completions, did-you-mean and the results view work as before.
 *
 * The field and the workspace query stay in step both ways: what the field holds is what filters (a message search
 * applies live, a query once run, and switching from one to the other drops the other), and a query or search set
 * from elsewhere (undo, a saved query, the assistant, a project) shows in the field.
 */
@Injectable({ providedIn: 'root' })
export class SearchField {
  private readonly backend = inject(AnalysisBackend);
  private readonly store = inject(WorkspaceStore);
  readonly session = inject(QuerySession);
  private readonly injector = inject(Injector);

  /** The field's text. */
  readonly text = this.session.text;
  /** What the text is, as the engine last said; a regular-expression search is always text. */
  readonly kind = signal<SearchInputKind>('text');
  /** The query is being classified (Enter waits for it). */
  readonly classifying = signal(false);
  /** A message search still applies while a query is shown (e.g. both set by an older project): shown as a chip. */
  readonly hiddenText = computed(() => {
    const q = this.store.query();
    return q.ql && q.text ? q.text : null;
  });
  /** The field holds a query that is not the one applied (Enter runs it). */
  readonly unrun = computed(() => this.kind() === 'query' && !!this.text().trim() && this.text().trim() !== (this.session.applied() ?? '').trim());

  private seq = 0;
  private pending: Promise<void> = Promise.resolve();
  private committed: Committed = { text: '', ql: '' };
  private started = false;

  /** Starts following the workspace query (idempotent; the field component calls it). */
  start(): void {
    if (this.started) return;
    this.started = true;
    effect(
      () => {
        const ql = this.session.applied() ?? '';
        const text = this.store.textInput();
        untracked(() => this.follow(ql, text));
      },
      { injector: this.injector },
    );
  }

  /** The text changed in the field (`cursorBytes`: the caret as a UTF-8 byte offset, for completions). */
  edit(text: string, cursorBytes: number): Promise<void> {
    this.text.set(text);
    this.session.runError.set(null);
    const mine = ++this.seq;
    const regex = this.store.query().regex;
    this.classifying.set(true);
    const classified = regex || !text.trim() ? Promise.resolve<SearchInputKind>('text') : this.backend.classifySearch(text, this.store.fieldNames()).catch((): SearchInputKind => 'text');
    this.pending = classified.then((kind) => {
      if (mine !== this.seq) return;
      this.classifying.set(false);
      this.apply(kind, text, cursorBytes);
    });
    return this.pending;
  }

  /** Enter: waits for the text to be classified, then runs a query (a message search already applies). */
  async submit(): Promise<'events' | 'table' | 'text' | null> {
    await this.pending;
    if (this.kind() !== 'query') return 'text';
    const text = this.text();
    // The run puts the text into the query and the search (if any) was dropped when the text became a query.
    this.committed = { text: '', ql: text };
    return this.session.run();
  }

  /** Empties the field and removes what it applied. */
  clear(): void {
    this.seq++;
    this.classifying.set(false);
    this.kind.set('text');
    this.text.set('');
    this.session.error.set(null);
    this.session.dismissCompletions();
    this.commitText('');
  }

  /** Removes the message search that still applies under a query (the chip's ×). */
  dropHiddenText(): void {
    this.committed = { text: '', ql: this.session.applied() ?? '' };
    this.store.setTextInput('');
  }

  private apply(kind: SearchInputKind, text: string, cursorBytes: number): void {
    this.kind.set(kind);
    if (kind === 'text') {
      this.session.error.set(null);
      this.session.dismissCompletions();
      this.commitText(text);
      return;
    }
    // A query: the message search it replaces stops filtering; the query filters once it is run.
    if (this.store.textInput()) {
      this.committed = { text: '', ql: this.session.applied() ?? '' };
      this.store.setTextInput('');
    }
    this.session.edit(text, cursorBytes);
  }

  /** Applies `text` as the message search (live), dropping a query that was applied. */
  private commitText(text: string): void {
    this.committed = { text, ql: '' };
    if (this.session.applied()) this.session.clearApplied();
    if (this.store.textInput() !== text) this.store.setTextInput(text);
  }

  /** The workspace query changed: show what now filters, unless the field made the change itself. */
  private follow(ql: string, text: string): void {
    if (ql === this.committed.ql && text === this.committed.text) return;
    this.committed = { text, ql };
    this.seq++;
    this.classifying.set(false);
    if (ql) {
      this.kind.set('query');
      if (this.text() !== ql) this.session.load(ql);
    } else {
      this.kind.set('text');
      this.session.error.set(null);
      this.session.dismissCompletions();
      if (this.text() !== text) this.text.set(text);
    }
  }
}
