import { ChangeDetectionStrategy, Component, computed, ElementRef, inject, OnInit, signal, viewChild } from '@angular/core';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { SavedSqlStore } from '../projects/bookmarks/saved-sql.store';
import { formatCount, formatDuration } from '../shared/format/units';
import { applyCompletion, byteToIndex, completeSql, rowsToTsv, SqlCompletion } from './sql-complete';
import { SqlConsoleStore } from './sql-console.store';
import { SqlExportFiles } from './sql-export-files';

/**
 * The SQL tab of the Store pane (ADR 0030): a monospace editor (`Ctrl+Enter` runs, `Esc` stops, `Tab` takes the
 * highlighted completion of a view, column or function the engine listed), an Explain switch, the engine's error with
 * the span it named, and the result: a table that pages on, Copy, and Open in Events for rows with event ids.
 */
@Component({
  selector: 'debrief-sql-console',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="editor-wrap">
      <textarea
        #editor
        class="editor mono"
        data-testid="sql-editor"
        spellcheck="false"
        aria-label="SQL query"
        i18n-aria-label="@@store.sql.editor"
        role="combobox"
        aria-autocomplete="list"
        [attr.aria-expanded]="completions().length > 0"
        aria-controls="sql-completions"
        [attr.aria-activedescendant]="completions().length ? 'sql-c-' + active() : null"
        [attr.aria-invalid]="!!sql.error()"
        aria-describedby="sql-error"
        [value]="sql.text()"
        (input)="onInput($event)"
        (keydown)="onKey($event)"
        (blur)="closeCompletions()"
      ></textarea>
      @if (completions().length) {
        <ul class="completions" id="sql-completions" role="listbox" aria-label="Completions" i18n-aria-label="@@store.sql.completions" data-testid="sql-completions">
          @for (c of completions(); track c.kind + c.text; let i = $index) {
            <li [id]="'sql-c-' + i" role="option" [attr.aria-selected]="i === active()" [class.active]="i === active()" (mousedown)="$event.preventDefault(); take(c)">
              <span class="mono">{{ c.text }}</span><span class="kind">{{ c.kind }}</span><span class="detail ellipsis">{{ c.detail }}</span>
            </li>
          }
        </ul>
      }
    </div>
    <div class="bar">
      @if (sql.running()) {
        <button type="button" class="btn" data-testid="sql-stop" (click)="sql.stop()" i18n="@@store.sql.stop">Stop</button>
        <span class="faint" role="status" i18n="@@store.sql.running">Running…</span>
      } @else {
        <button type="button" class="btn primary" data-testid="sql-run" (click)="sql.run()" title="Ctrl+Enter" i18n="@@store.sql.run">Run</button>
      }
      <label class="check"><input type="checkbox" data-testid="sql-explain" [checked]="sql.explain()" (change)="sql.setExplain(checked($event))" /> <ng-container i18n="@@store.sql.explain">Explain</ng-container></label>
      @if (sql.history().length) {
        <select class="input" aria-label="Earlier queries" i18n-aria-label="@@store.sql.history" (change)="fromHistory($event)">
          <option value="" i18n="@@store.sql.historyPlaceholder">History…</option>
          @for (h of sql.history(); track h; let i = $index) {
            <option [value]="i">{{ h.slice(0, 80) }}</option>
          }
        </select>
      }
      <span class="grow"></span>
      <input class="input name" aria-label="Name for this SQL" i18n-aria-label="@@store.sql.saveName" placeholder="Name" i18n-placeholder="@@store.sql.saveNamePlaceholder" [value]="saveName()" (input)="saveName.set(value($event))" />
      <button type="button" class="btn" data-testid="sql-save" (click)="save()" i18n="@@store.sql.save">Save SQL</button>
    </div>
    <div id="sql-error" aria-live="polite">
      @if (sql.error(); as e) {
        <div class="error" role="alert" data-testid="sql-error">
          <div>{{ e.message }}</div>
          @if (errorSpan(); as s) {
            <pre class="mono snippet">{{ s.before }}<mark>{{ s.marked || ' ' }}</mark>{{ s.after }}</pre>
          }
        </div>
      }
      @if (sql.failure(); as f) {
        <div class="error" role="alert" data-testid="sql-failure">{{ f }}</div>
      }
      @if (sql.notice(); as n) {
        <div class="faint" role="status" data-testid="sql-notice">{{ n }}</div>
      }
      @if (sql.exportJob() !== null) {
        <div class="faint" role="status" data-testid="sql-export-started" i18n="@@store.sql.exportStarted">Exporting in the background: follow it in Jobs.</div>
      }
      @if (sql.stopped()) {
        <div class="faint" role="status" i18n="@@store.sql.stopped">Stopped.</div>
      }
    </div>
    @if (sql.result(); as r) {
      @if (r.plan !== null) {
        <pre class="plan mono" data-testid="sql-plan" tabindex="0" aria-label="Query plan" i18n-aria-label="@@store.sql.plan">{{ r.plan }}</pre>
      } @else {
        <div class="bar">
          <span data-testid="sql-count">{{ countText() }}</span>
          <span class="faint">{{ elapsed() }}</span>
          <span class="grow"></span>
          @if (r.eventIds) {
            <button type="button" class="btn" data-testid="sql-open-events" [disabled]="!eventId()" (click)="openInEvents()" i18n="@@store.sql.openInEvents">Open in Events</button>
          }
          @if (r.eventIds) {
            <button type="button" class="btn" data-testid="sql-open-all-events" (click)="openAllInEvents()" i18n="@@store.sql.openAllInEvents">Open all in Events</button>
          }
          <button type="button" class="btn" data-testid="sql-export" [disabled]="!!sql.runId()" (click)="exportCsv()" i18n="@@store.sql.exportCsv">Export CSV</button>
          <button type="button" class="btn" (click)="copy()" i18n="@@store.sql.copy">Copy</button>
        </div>
        <div class="table-wrap" tabindex="0" role="region" aria-label="Results" i18n-aria-label="@@store.sql.results">
          <table class="results" data-testid="sql-results">
            <thead>
              <tr>
                @for (c of r.columns; track $index) {
                  <th scope="col" [title]="c.type">{{ c.name }}</th>
                }
              </tr>
            </thead>
            <tbody>
              @for (row of sql.rows(); track $index; let i = $index) {
                <tr [class.selected]="i === selected()" (click)="selected.set(i)" (dblclick)="selected.set(i); openInEvents()">
                  @for (cell of row; track $index) {
                    <td [class.null]="cell === null">{{ cell === null ? 'NULL' : cell }}</td>
                  }
                </tr>
              }
            </tbody>
          </table>
        </div>
        @if (sql.hasMore()) {
          <button type="button" class="btn more" data-testid="sql-more" [disabled]="sql.loadingMore()" (click)="sql.loadMore()" i18n="@@store.sql.more">Show more rows</button>
        }
      }
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; min-height: 0; }
    .editor-wrap { position: relative; }
    .editor { width: 100%; min-height: 120px; resize: vertical; box-sizing: border-box; padding: 8px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); color: var(--text); font-size: var(--fs-sm); }
    .editor[aria-invalid='true'] { border-color: var(--lvl-error); }
    .completions { position: absolute; left: 8px; bottom: -4px; transform: translateY(100%); z-index: 5; list-style: none; margin: 0; padding: 4px 0; min-width: 260px; max-width: 480px; background: var(--bg-3); border: 1px solid var(--border-3); border-radius: 6px; box-shadow: var(--shadow); }
    .completions li { display: flex; gap: 8px; padding: 2px 10px; cursor: pointer; font-size: var(--fs-sm); }
    .completions li.active { background: var(--bg-sel); }
    .kind { color: var(--muted); font-size: var(--fs-xs); }
    .detail { color: var(--faint, var(--muted)); font-size: var(--fs-xs); min-width: 0; flex: 1; }
    .bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .grow { flex: 1; }
    .name { width: 140px; }
    .check { display: flex; align-items: center; gap: 4px; }
    .error { color: var(--lvl-error); display: flex; flex-direction: column; gap: 4px; }
    .snippet { margin: 0; padding: 4px 8px; background: var(--bg-2); border-radius: 4px; color: var(--text-2); white-space: pre-wrap; overflow-wrap: anywhere; }
    mark { background: transparent; color: inherit; text-decoration: underline wavy var(--lvl-error); }
    .plan { margin: 0; padding: 8px; background: var(--bg-2); border-radius: 6px; overflow: auto; max-height: 420px; font-size: var(--fs-xs); }
    .table-wrap { overflow: auto; max-height: 420px; border: 1px solid var(--border-2); border-radius: 6px; }
    .results { border-collapse: collapse; width: max-content; min-width: 100%; font-size: var(--fs-sm); }
    th, td { text-align: left; padding: 3px 8px; border-bottom: 1px solid var(--border-2); white-space: nowrap; max-width: 480px; overflow: hidden; text-overflow: ellipsis; }
    th { position: sticky; top: 0; background: var(--bg-2); font-weight: 600; }
    tr.selected td { background: var(--bg-sel); }
    td.null { color: var(--muted); font-style: italic; }
    .more { align-self: flex-start; }
  `,
})
export class SqlConsole implements OnInit {
  protected readonly sql = inject(SqlConsoleStore);
  private readonly saved = inject(SavedSqlStore);
  private readonly ctx = inject(COMMAND_CONTEXT);
  private readonly files = inject(SqlExportFiles);
  private readonly editor = viewChild<ElementRef<HTMLTextAreaElement>>('editor');
  protected readonly completions = signal<SqlCompletion[]>([]);
  protected readonly active = signal(0);
  protected readonly selected = signal(0);
  protected readonly saveName = signal('');

  /** The engine's error span in the editor's text (offsets are UTF-8 bytes). */
  protected readonly errorSpan = computed(() => {
    const e = this.sql.error();
    if (!e) return null;
    const text = this.sql.text();
    const start = byteToIndex(text, e.offset);
    const end = byteToIndex(text, e.offset + e.len);
    const lineStart = text.lastIndexOf('\n', start - 1) + 1;
    const nl = text.indexOf('\n', end);
    const lineEnd = nl < 0 ? text.length : nl;
    return { before: text.slice(lineStart, start), marked: text.slice(start, end), after: text.slice(end, lineEnd) };
  });

  protected readonly countText = computed(() => {
    const r = this.sql.result();
    if (!r) return '';
    const shown = formatCount(this.sql.rows().length);
    const total = formatCount(r.total);
    return r.truncated
      ? $localize`:@@store.sql.countCut:${shown}:shown: of ${total}:total: rows (the query returned more; the console keeps ${total}:max:)`
      : $localize`:@@store.sql.count:${shown}:shown: of ${total}:total: rows`;
  });
  protected readonly elapsed = computed(() => formatDuration(this.sql.result()?.elapsedMs ?? 0));

  /** The event id of the selected row (the result's `id` column). */
  protected readonly eventId = computed(() => {
    const r = this.sql.result();
    if (!r?.eventIds) return null;
    const col = r.columns.findIndex((c) => c.name.toLowerCase() === 'id');
    const v = this.sql.rows()[this.selected()]?.[col];
    return typeof v === 'number' ? v : null;
  });

  ngOnInit(): void {
    void this.sql.loadSchema();
  }

  protected onInput(e: Event): void {
    const el = e.target as HTMLTextAreaElement;
    this.sql.setText(el.value);
    this.completions.set(completeSql(el.value, el.selectionStart ?? el.value.length, this.sql.schema()));
    this.active.set(0);
  }

  protected onKey(e: KeyboardEvent): void {
    const list = this.completions();
    if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
      e.preventDefault();
      this.closeCompletions();
      void this.sql.run();
    } else if (e.key === 'Escape') {
      if (list.length) this.closeCompletions();
      else if (this.sql.running()) void this.sql.stop();
      else return;
      e.preventDefault();
    } else if (list.length && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
      e.preventDefault();
      const step = e.key === 'ArrowDown' ? 1 : -1;
      this.active.set((this.active() + step + list.length) % list.length);
    } else if (list.length && e.key === 'Tab') {
      e.preventDefault();
      const c = list[this.active()];
      if (c) this.take(c);
    }
  }

  protected take(c: SqlCompletion): void {
    const el = this.editor()?.nativeElement;
    const cursor = el?.selectionStart ?? this.sql.text().length;
    const next = applyCompletion(this.sql.text(), cursor, c.text);
    this.sql.setText(next.text);
    this.closeCompletions();
    if (el) {
      el.value = next.text;
      el.setSelectionRange(next.cursor, next.cursor);
      el.focus();
    }
  }

  protected closeCompletions(): void {
    this.completions.set([]);
  }

  protected fromHistory(e: Event): void {
    const select = e.target as HTMLSelectElement;
    const text = this.sql.history()[Number(select.value)];
    if (text !== undefined && select.value !== '') this.sql.open(text);
    select.value = '';
  }

  protected save(): void {
    this.saved.save(this.saveName(), this.sql.text());
    this.saveName.set('');
  }

  protected openInEvents(): void {
    const id = this.eventId();
    if (id !== null) void this.ctx.jumpTo(id);
  }

  protected async openAllInEvents(): Promise<void> {
    const ql = await this.sql.idsQuery();
    if (ql === null) return;
    this.ctx.setQuery({ ql });
    this.ctx.showView('events');
  }

  protected async exportCsv(): Promise<void> {
    const path = await this.files.pickCsv();
    if (path) await this.sql.exportCsv(path);
  }

  protected copy(): void {
    const r = this.sql.result();
    if (!r) return;
    void navigator.clipboard?.writeText(rowsToTsv(r.columns.map((c) => c.name), this.sql.rows())).catch(() => undefined);
  }

  protected checked = (e: Event) => (e.target as HTMLInputElement).checked;
  protected value = (e: Event) => (e.target as HTMLInputElement).value;
}
