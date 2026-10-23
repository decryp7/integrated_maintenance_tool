import { DialogRef } from '@angular/cdk/dialog';
import { ChangeDetectionStrategy, Component, computed, inject, Injector } from '@angular/core';
import { ValueMatch } from '../core/api.types';
import { formatCount, formatTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { ActiveDescendantNavigation } from '../shared/list-navigation';
import { ResultRow } from './search-everything.model';
import { SearchEverythingNavigator } from './search-everything-navigator';
import { SearchEverythingStore } from './search-everything.store';

let nextId = 0;
/** Spellings listed under a suggestion. */
const MAX_SPELLINGS_SHOWN = 4;

/**
 * The search everything palette (Mod+Shift+F), shown in a CDK dialog: a combobox whose results are grouped by kind with
 * the matched words marked; Enter or a click goes to the result. The engine searches; this only shows and navigates.
 */
@Component({
  selector: 'debrief-search-everything',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { 'data-activity-trigger': 'palette', class: 'debrief-search-everything' },
  template: `
    <input
      class="input"
      role="combobox"
      data-testid="search-everything-input"
      i18n-aria-label="@@search.input.label"
      aria-label="Search everything"
      aria-autocomplete="list"
      aria-expanded="true"
      [attr.aria-controls]="listId"
      [attr.aria-activedescendant]="activeId()"
      i18n-placeholder="@@search.input.placeholder"
      placeholder="Search events, sources, fields, markers, notes…"
      autocomplete="off"
      spellcheck="false"
      [value]="store.text()"
      (input)="onInput($any($event.target).value)"
      (keydown)="onKey($event)"
    />
    <div class="status" aria-live="polite">
      @if (store.loading()) {
        <span class="muted" i18n="@@search.status.searching">Searching…</span>
      } @else if (store.error(); as error) {
        <span class="err" data-testid="search-everything-error">{{ error }}</span>
        <button type="button" class="btn ghost" (click)="store.searchNow()" i18n="@@search.status.retry">Retry</button>
      } @else if (store.searched().trim() && !store.rows().length && !store.groups().length) {
        <span class="muted" data-testid="search-everything-empty" i18n="@@search.status.none">Nothing matches “{{ store.searched() //i18n(ph="text") }}”.</span>
      }
    </div>
    @if (store.clarify(); as c) {
      <div class="status ask" data-testid="search-everything-clarify" role="group" i18n-aria-label="@@search.clarify.label" aria-label="Which one do you mean?">
        <span class="muted">{{ c.question }}</span>
        @for (o of store.options(); track o.group) {
          <button type="button" class="btn ghost" [attr.data-testid]="'search-everything-option'" (click)="store.acceptName(o)" [title]="why(o)">{{ o.value }}</button>
        }
      </div>
    } @else if (store.suggestion(); as s) {
      <div class="status" data-testid="search-everything-suggestion">
        <span class="muted" i18n="@@search.suggestion.prefix">Did you mean</span>
        <button type="button" class="btn ghost" (click)="store.acceptSuggestion()">{{ s.value }}</button>
        <span class="muted">({{ fieldName(s.field, s.kind) }})?</span>
        @if (s.group) {
          <button type="button" class="btn ghost" data-testid="search-everything-accept" (click)="store.acceptName(s)" i18n="@@search.suggestion.accept">Yes, remember</button>
          <button type="button" class="btn ghost" data-testid="search-everything-reject" (click)="store.rejectName(s)" i18n="@@search.suggestion.reject">Not this</button>
        }
      </div>
      @if (s.why) {
        <div class="status why" data-testid="search-everything-why">
          <span class="muted">{{ s.why }}</span>
          @if (s.via === 'alias') {
            <span class="tag" data-testid="search-everything-ai" i18n="@@search.suggestion.ai">AI-generated</span>
          }
          @if (spellings(s); as written) {
            <span class="muted" data-testid="search-everything-spellings" i18n="@@search.suggestion.spellings">Also written: {{ written //i18n(ph="spellings") }}</span>
          }
        </div>
      }
    }
    @if (store.feedback(); as f) {
      <div class="status" data-testid="search-everything-feedback" aria-live="polite">
        @if (f.kind === 'accepted') {
          <span class="muted" i18n="@@search.feedback.accepted">Remembered: “{{ f.typed //i18n(ph="typed") }}” means {{ f.value //i18n(ph="value") }}.</span>
        } @else {
          <span class="muted" i18n="@@search.feedback.rejected">Remembered: “{{ f.typed //i18n(ph="typed") }}” is not {{ f.value //i18n(ph="value") }}.</span>
        }
        <button type="button" class="btn ghost" data-testid="search-everything-undo" (click)="store.undoName()" i18n="@@search.feedback.undo">Undo</button>
      </div>
    }
    <div class="list" role="listbox" [id]="listId" i18n-aria-label="@@search.list.label" aria-label="Results">
      @for (g of store.groups(); track g.kind) {
        <div class="group" role="group" [attr.aria-labelledby]="listId + '-' + g.kind" [attr.data-testid]="'search-group-' + g.kind">
          <div class="head" role="presentation" [id]="listId + '-' + g.kind">
            <span class="title">{{ g.title }}</span>
            <span class="count">{{ count(g.total) }}</span>
          </div>
          @if (g.notice) {
            <div class="notice" role="presentation">{{ g.notice }}</div>
          }
          @for (r of g.rows; track r.key) {
            <div
              role="option"
              class="opt"
              [id]="optionId(r)"
              [class.active]="r === activeRow()"
              [attr.aria-selected]="r === activeRow()"
              (mousemove)="nav.setActive(indexOf(r))"
              (click)="choose(r)"
            >
              <span class="snippet">@for (p of r.snippet; track $index) {@if (p.mark) {<mark>{{ p.text }}</mark>} @else {{{ p.text }}}}</span>
              @if (r.ts !== null) { <span class="time">{{ time(r.ts) }}</span> }
              @if (r.detail) { <span class="detail">{{ r.detail }}</span> }
            </div>
          }
        </div>
      }
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 6px; width: min(680px, calc(100vw - 32px)); max-height: min(560px, calc(100vh - 120px)); padding: 8px; border-radius: 10px; background: var(--tooltip-bg); border: 1px solid var(--border-3); box-shadow: var(--shadow); color: var(--text); }
    .input { height: 32px; }
    .status { display: flex; align-items: center; gap: 8px; min-height: 0; font-size: var(--fs-sm); }
    .status:empty { display: none; }
    .muted { color: var(--muted); padding: 2px 4px; }
    .status.ask, .status.why { flex-wrap: wrap; }
    .tag { padding: 0 6px; border: 1px solid var(--border-3); border-radius: 8px; color: var(--muted); font-size: var(--fs-xs); }
    .err { color: var(--danger-text); padding: 2px 4px; }
    .list { overflow: auto; display: flex; flex-direction: column; gap: 4px; min-height: 0; }
    .head { display: flex; align-items: baseline; gap: 6px; padding: 6px 10px 2px; color: var(--muted); font-size: var(--fs-xs); }
    .count { font-variant-numeric: tabular-nums; }
    .notice { padding: 0 10px 2px; color: var(--muted); font-size: var(--fs-sm); font-style: italic; }
    .opt { display: flex; align-items: center; gap: 8px; min-height: 30px; padding: 2px 10px; border-radius: 6px; color: var(--text-2); cursor: pointer; }
    .opt.active { background: var(--bg-active); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
    .snippet { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .snippet mark { background: none; color: var(--accent-text); font-weight: 600; }
    .time, .detail { flex: none; color: var(--muted); font-size: var(--fs-sm); max-width: 40%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .time { font-family: var(--font-mono); font-size: var(--fs-xs); }
  `,
})
export class SearchEverythingPalette {
  protected readonly store = inject(SearchEverythingStore);
  protected readonly count = formatCount;
  private readonly workspace = inject(WorkspaceStore);
  private readonly navigator = inject(SearchEverythingNavigator);
  private readonly dialog = inject(DialogRef, { optional: true });
  private readonly injector = inject(Injector);

  protected readonly listId = `debrief-search-everything-${nextId++}`;
  /** The highlighted result (CDK ActiveDescendantKeyManager; focus stays in the input). */
  protected readonly nav = new ActiveDescendantNavigation(
    computed(() => this.store.rows().length),
    this.injector,
    { wrap: true, selectFirst: true },
  );
  protected readonly activeRow = computed<ResultRow | null>(() => this.store.rows()[this.nav.active()] ?? null);
  protected readonly activeId = computed(() => {
    const r = this.activeRow();
    return r ? this.optionId(r) : null;
  });

  constructor() {
    this.store.reset();
  }

  /** A field as shown: the pseudo field of a source's name reads "source". */
  protected fieldName(field: string, kind?: string): string {
    if (field === '@entity' && kind) return kind;
    return field === '@source' ? $localize`:@@search.suggestion.source:source` : field;
  }

  /** The other spellings of the suggested name, a few of them; empty when the logs write it one way. */
  protected spellings(v: ValueMatch): string {
    const others = (v.spellings ?? []).filter((s) => s.name !== v.value).map((s) => s.name);
    const shown = others.slice(0, MAX_SPELLINGS_SHOWN).join(', ');
    return others.length > MAX_SPELLINGS_SHOWN ? `${shown}, …` : shown;
  }

  /** Why a value matched, for a tooltip. */
  protected why(v: ValueMatch): string {
    return v.why;
  }

  protected optionId(r: ResultRow): string {
    return `${this.listId}-${r.key.replace(/[^\w-]/g, '_')}-${this.indexOf(r)}`;
  }

  protected indexOf(r: ResultRow): number {
    return this.store.rows().indexOf(r);
  }

  protected time(ts: number): string {
    return formatTime(ts, this.workspace.timezone());
  }

  protected onInput(value: string): void {
    this.store.setText(value);
    this.nav.setActive(0);
  }

  protected onKey(e: KeyboardEvent): void {
    switch (e.key) {
      case 'ArrowDown':
      case 'ArrowUp':
        e.preventDefault();
        this.nav.handle(e);
        break;
      case 'Enter': {
        e.preventDefault();
        const r = this.activeRow();
        if (r && this.store.searched() === this.store.text()) this.choose(r);
        else void this.store.searchNow();
        break;
      }
    }
  }

  protected choose(r: ResultRow): void {
    this.dialog?.close();
    this.navigator.go(r.target);
  }
}
