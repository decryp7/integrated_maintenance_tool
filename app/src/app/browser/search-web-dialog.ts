import { ChangeDetectionStrategy, Component, computed, inject, input, OnInit, signal } from '@angular/core';
import { DIALOG_REF } from '../ui/dialog.service';
import { BrowserBackend } from './browser-backend';
import { BrowserStore } from './browser.store';
import { QuerySource, SearchEngine } from './browser.types';
import { engineLabel, ENGINE_OPTIONS } from './browser-text';

/**
 * "Search the web for this error": the AI drafts the search text from the event (the engine builds it without a model);
 * what the leak guard flags in it, names from the logs
 * and private details, is removed. The user checks or edits it and picks the engine before anything opens.
 */
@Component({
  selector: 'imt-search-web-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (loading()) {
      <p class="muted" role="status" i18n="@@browser.searchDialog.loading">Preparing the search…</p>
    } @else {
      <form (submit)="$event.preventDefault(); search()">
        <label for="web-search-text" i18n="@@browser.searchDialog.label">Search for</label>
        <input id="web-search-text" class="input" maxlength="500" [value]="query()" (input)="query.set(asValue($event))" data-testid="web-search-text" />
        @if (source(); as s) {
          <p class="muted small" data-testid="web-search-source">{{ s === 'model' ? byModel : byEngine }}</p>
        }
        @if (removed().length) {
          <p class="muted small" data-testid="web-search-removed">{{ removedText() }}</p>
        }
        <p class="muted small" i18n="@@browser.searchDialog.note">Only this text goes to the search engine, in IMT's Browser panel. Nothing else from your logs leaves this computer.</p>
        <div class="row">
          <label for="web-search-engine" i18n="@@browser.searchDialog.engine">Engine</label>
          <select id="web-search-engine" class="select" [value]="engine()" (change)="setEngine(asValue($event))">
            @for (e of engines(); track e) {
              <option [value]="e">{{ engineName(e) }}</option>
            }
          </select>
          <span class="spacer"></span>
          <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@browser.searchDialog.cancel">Cancel</button>
          <button type="submit" class="btn primary" [disabled]="!query().trim() || busy()" i18n="@@browser.searchDialog.search">Search</button>
        </div>
      </form>
    }
    @if (error(); as e) {
      <p class="err" role="alert">{{ e }} <button type="button" class="link" (click)="retry()" i18n="@@browser.searchDialog.retry">Retry</button></p>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; }
    form { display: flex; flex-direction: column; gap: 6px; }
    p { margin: 0; }
    .small { font-size: var(--fs-sm); }
    .row { display: flex; align-items: center; gap: 8px; margin-top: 4px; }
    .spacer { flex: 1; }
    .err { color: var(--danger-text, var(--lvl-error)); }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); }
  `,
})
export class SearchWebDialog implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(BrowserBackend);
  private readonly store = inject(BrowserStore);

  /** The event to search for; or `text` to start from. */
  readonly recordId = input<number | null>(null);
  readonly text = input('');

  protected readonly query = signal('');
  protected readonly removed = signal<string[]>([]);
  /** Who wrote the text (null: the user, or not prepared yet). */
  protected readonly source = signal<QuerySource | null>(null);
  protected readonly byModel = $localize`:@@browser.searchDialog.byModel:The AI wrote this search from the event.`;
  protected readonly byEngine = $localize`:@@browser.searchDialog.byEngine:IMT built this search from the event (the AI could not write it).`;
  /** Null until the browser state is known: the backend then searches with the engine from Settings. */
  protected readonly engine = signal<SearchEngine | null>(null);
  protected readonly loading = signal(false);
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly engines = computed(() => ENGINE_OPTIONS.filter((e) => e !== 'custom' || this.store.customEngine()));
  protected readonly removedText = computed(() => $localize`:@@browser.searchDialog.removed:Removed for your privacy: ${this.removed().join(', ')}:what:.`);
  private failed: (() => Promise<void>) | null = null;

  ngOnInit(): void {
    this.engine.set(this.store.engine());
    this.query.set(this.text());
    void this.prepare();
  }

  protected asValue(e: Event): string {
    return (e.target as HTMLInputElement | HTMLSelectElement).value;
  }

  protected engineName(e: SearchEngine): string {
    return engineLabel(e);
  }

  protected setEngine(v: string): void {
    if (ENGINE_OPTIONS.includes(v as SearchEngine)) this.engine.set(v as SearchEngine);
  }

  async prepare(): Promise<void> {
    const id = this.recordId();
    if (id === null) return;
    this.loading.set(true);
    this.error.set(null);
    try {
      if (!this.store.loaded()) await this.store.load();
      this.engine.set(this.store.engine());
      const q = await this.backend.eventQuery(id);
      this.query.set(q.query);
      this.removed.set(q.removed);
      this.source.set(q.source);
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
      this.failed = () => this.prepare();
    } finally {
      this.loading.set(false);
    }
  }

  async search(): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    try {
      const engine = this.engine();
      if (engine) this.store.setEngine(engine);
      if (await this.store.open({ kind: 'search', query: this.query(), engine: this.engine() })) this.ref.close(true);
      else {
        this.error.set(this.store.error());
        this.failed = () => this.search();
      }
    } finally {
      this.busy.set(false);
    }
  }

  async retry(): Promise<void> {
    await this.failed?.();
  }
}
