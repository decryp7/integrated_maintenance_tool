import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, Injector, output, signal, untracked, viewChild } from '@angular/core';
import { ClickOutside } from '../shared/click-outside';
import { ActiveDescendantNavigation } from '../shared/list-navigation';
import { Completion, QueryProgress } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { SmartSearchStore } from '../core/workspace/smart-search.store';
import { formatKeybinding } from '../extensions/keybinding';
import { groupedNumber, plural } from '../i18n/locale';
import { Icon } from '../shared/icon';
import { applyCompletion, applySuggestion, charToByte, errorSpan, highlight } from './ql-text';
import { QlSuggestions } from './ql-suggestions';
import { SearchField } from './search-field';
import { AiLauncher } from '../ai/ai-launcher';
import { AiStatusStore } from '../ai/ai-status.store';
import { SavedQueryStore } from '../projects/bookmarks/saved-query.store';
import { CommandRegistry } from '../extensions/command-registry';

/** Lines the field grows to while focused before it scrolls. */
export const MAX_FIELD_LINES = 6;
/** Line height of the field, px (matches the CSS). */
const LINE_PX = 20;

/** "Matching 1,200 of 50,000 events (2%)" for the status line; null before the engine reports anything. */
export function progressText(p: QueryProgress | null): string | null {
  if (!p || p.total <= 0) return null;
  const pct = Math.min(100, Math.floor((p.done / p.total) * 100));
  const done = groupedNumber(p.done);
  const total = groupedNumber(p.total);
  return p.stage === 'matching'
    ? $localize`:@@analysis.queryView.progress.matching:Matching ${done}:done: of ${total}:total: events (${pct}:pct:%)`
    : $localize`:@@analysis.queryView.progress.evaluating:Evaluating ${done}:done: of ${total}:total: events (${pct}:pct:%)`;
}

/** Focuses the element, retrying while it is still being shown (the bar may be laid out after the command ran). */
export function focusSoon(el: HTMLElement, attempts = 20): void {
  el.focus();
  if (document.activeElement !== el && attempts > 0) setTimeout(() => focusSoon(el, attempts - 1), 50);
}

/**
 * The filter bar's one search field (D2): type words to search the messages, or a query (`level >= warning | stats
 * count() by source`) — the engine tells which. A query is highlighted, checked as you type (the error and its
 * did-you-mean show under the bar), completed (Ctrl+Space) and run with Enter or Ctrl+Enter; the field grows to
 * several lines while focused (Shift+Enter for a new line). History, Ask AI and Save sit inside it.
 */
@Component({
  selector: 'debrief-query-field',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ClickOutside, Icon],
  host: { '[class.focused]': 'focused()', '[class.query]': "field.kind() === 'query'" },
  template: `
    <div class="box" [class.invalid]="invalid()" [class.expanded]="expanded()" (debriefClickOutside)="closePopups()">
      <debrief-icon [name]="field.kind() === 'query' ? 'analysis' : 'search'" [size]="14" class="faint lead" />
      <div class="editor">
        @if (field.kind() === 'query') {
          <div class="mirror mono" aria-hidden="true" [class.wrap]="expanded()"><div class="shift" [style.transform]="shift()">@for (s of segments(); track $index) {<span [class]="s.cls" [class.err]="s.err">{{ s.text }}</span>}@if (caretError()) {<span class="err-caret"></span>}</div></div>
        }
        <textarea
          #input
          class="qinput mono"
          role="combobox"
          rows="1"
          [class.wrap]="expanded()"
          [class.see-through]="field.kind() === 'query'"
          aria-label="Search messages and exceptions, or type a query" i18n-aria-label="@@analysis.queryField.label"
          aria-autocomplete="list"
          autocomplete="off"
          spellcheck="false"
          data-testid="query-field"
          [placeholder]="placeholder()"
          [attr.aria-expanded]="popupOpen()"
          [attr.aria-controls]="popupOpen() ? 'ql-completions' : null"
          [attr.aria-activedescendant]="popupOpen() ? 'ql-opt-' + active() : null"
          [attr.aria-invalid]="invalid() ? 'true' : null"
          [attr.aria-describedby]="describedBy()"
          [value]="field.text()"
          (input)="onInput()"
          (keydown)="onKey($event)"
          (keyup)="sync()"
          (click)="sync()"
          (scroll)="sync()"
          (focus)="onFocus()"
          (blur)="onBlur()"
        ></textarea>
        @if (popupOpen()) {
          <ul class="popup" id="ql-completions" role="listbox" aria-label="Completions" i18n-aria-label="@@analysis.queryView.completions">
            @for (c of session.completions(); track c.label + $index; let i = $index) {
              <li [id]="'ql-opt-' + i" role="option" [attr.aria-selected]="i === active()" [class.active]="i === active()" (mousedown)="$event.preventDefault()" (click)="accept(c)">
                <span class="kind">{{ c.kind }}</span><span class="mono">{{ c.label }}</span>
              </li>
            }
          </ul>
        }
      </div>
      @if (field.kind() === 'text' && smart.added() > 0) {
        <span class="partial-note mono" role="status" data-testid="partial-note" [title]="partialNote()">+{{ smart.added() }}</span>
      }
      @if (field.kind() === 'text') {
        <button type="button" class="toggle mono" [class.on]="q().regex" (click)="store.setRegex(!q().regex)" title="Regular expression" i18n-title="@@workspace.filterBar.regex" [attr.aria-pressed]="q().regex">.*</button>
        <button type="button" class="toggle mono" [class.on]="q().caseSensitive" (click)="store.setCaseSensitive(!q().caseSensitive)" title="Match case" i18n-title="@@workspace.filterBar.matchCase" [attr.aria-pressed]="q().caseSensitive">Aa</button>
      }
      <div class="anchor" (debriefClickOutside)="historyOpen.set(false)">
        <button type="button" class="ib" data-testid="query-history" aria-label="Query history" i18n-aria-label="@@analysis.queryField.history" title="Query history (Alt+↑ / Alt+↓)" i18n-title="@@analysis.queryField.historyTitle"
                (click)="historyOpen.set(!historyOpen())" [attr.aria-expanded]="historyOpen()" aria-haspopup="true" [disabled]="!session.history.entries().length">
          <debrief-icon name="refresh" [size]="13" />
        </button>
        @if (historyOpen()) {
          <ul class="popup history" aria-label="Query history" i18n-aria-label="@@analysis.queryView.historyLabel">
            @for (h of session.history.entries(); track h) {
              <li><button class="item mono" type="button" (click)="useHistory(h)">{{ h }}</button></li>
            }
            <li class="clear"><button class="item" type="button" (click)="session.history.clear(); historyOpen.set(false)" i18n="@@analysis.queryView.clearHistory">Clear history</button></li>
          </ul>
        }
      </div>
      @if (aiStatus.available()) {
        <button type="button" class="ib" data-testid="ai-ask-query" aria-label="Ask AI to write a query" i18n-aria-label="@@analysis.queryField.askAi" title="Describe what you want in plain words; the local model writes the query" i18n-title="@@analysis.queryView.askAiHint" (click)="ai.askQuery()">
          <span class="spark" aria-hidden="true">✦</span>
        </button>
      }
      <button type="button" class="ib" data-testid="query-save" aria-label="Save as a saved query" i18n-aria-label="@@analysis.queryField.save" title="Save the current search and filters as a saved query" i18n-title="@@analysis.queryField.saveTitle" [disabled]="!field.text().trim()" (click)="save()">
        <debrief-icon name="note" [size]="13" />
      </button>
      @if (field.kind() === 'query') {
        @if (session.running()) {
          <button type="button" class="ib" data-testid="query-cancel" aria-label="Cancel the running query" i18n-aria-label="@@analysis.queryField.cancel" title="Cancel the running query (Esc)" i18n-title="@@analysis.queryView.cancelTitle" [disabled]="session.cancelling()" (click)="session.cancel()">
            <debrief-icon name="stop" [size]="13" />
          </button>
        } @else {
          <button type="button" class="run" data-testid="query-run" [class.pending]="field.unrun()" [title]="runTitle" (click)="submit()" i18n="Run a query|Button@@analysis.queryView.run">Run</button>
        }
      }
    </div>
  `,
  styles: `
    /* Basis 300 px: the bar wraps before the field falls below a usable width, so the placeholder is never cut. */
    :host { position: relative; flex: 1 1 340px; min-width: 340px; height: 30px; display: block; }
    .box {
      position: absolute; inset: 0 0 auto 0; min-height: 30px; box-sizing: border-box; z-index: 5;
      display: flex; align-items: flex-start; gap: 6px; padding: 0 4px 0 10px; border-radius: 6px; background: var(--bg-3); border: 1px solid var(--border-2);
    }
    .box:focus-within { border-color: var(--accent); }
    .box.expanded { box-shadow: var(--shadow); }
    .box.invalid { border-color: var(--danger-text); }
    .lead { flex: none; margin-top: 7px; }
    .editor { position: relative; flex: 1; min-width: 0; align-self: stretch; }
    .mirror, .qinput { box-sizing: border-box; margin: 0; padding: 4px 0; font: var(--fs-sm) / ${LINE_PX}px var(--font-mono); white-space: pre; overflow-wrap: normal; }
    .mirror.wrap, .qinput.wrap { white-space: pre-wrap; overflow-wrap: anywhere; }
    .mirror { position: absolute; inset: 0; overflow: hidden; pointer-events: none; color: var(--text); }
    .qinput {
      display: block; width: 100%; height: 28px; resize: none; overflow: hidden; border: 0; outline: 0; background: transparent; color: var(--text); caret-color: var(--text);
    }
    .qinput.wrap { overflow-y: auto; }
    .qinput.see-through { color: transparent; }
    .qinput.see-through::selection { background: var(--accent-soft); color: transparent; }
    .qinput::placeholder { color: var(--faint); }
    .kw { color: var(--chip-text); font-weight: 600; }
    .fn, .num { color: var(--ok); }
    .field { color: var(--accent-text); }
    .str { color: var(--warn-text); }
    .op, .punct { color: var(--muted); }
    .text { color: var(--text); }
    .err { text-decoration: underline wavy var(--danger-text); text-underline-offset: 3px; }
    .err-caret { display: inline-block; width: 8px; height: 14px; border-bottom: 2px solid var(--danger-text); vertical-align: bottom; }
    .partial-note { flex: none; margin-top: 6px; font-size: var(--fs-xs); color: var(--text-2); padding: 1px 6px; border-radius: 999px; border: 1px dashed var(--border-2); }
    .toggle { flex: none; margin-top: 4px; height: 20px; padding: 0 5px; font-size: var(--fs-2xs); border-radius: 4px; color: var(--muted); border: 1px solid var(--border-3); background: transparent; cursor: pointer; }
    .toggle.on { background: var(--accent); border-color: var(--accent); color: var(--accent-ink); }
    .ib { flex: none; margin-top: 2px; width: 24px; height: 24px; display: inline-flex; align-items: center; justify-content: center; border: 0; border-radius: 4px; background: transparent; color: var(--muted); cursor: pointer; }
    .ib:hover:not(:disabled) { background: var(--bg-hover); color: var(--text); }
    .ib:disabled { opacity: 0.45; cursor: default; }
    .ib:focus-visible, .toggle:focus-visible, .run:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .spark { color: var(--accent-text); font-size: var(--fs-xs); }
    .run { flex: none; margin-top: 2px; height: 24px; padding: 0 10px; border-radius: 4px; border: 1px solid var(--border-3); background: var(--bg-2); color: var(--text-2); font-size: var(--fs-sm); cursor: pointer; }
    .run.pending { background: var(--accent); border-color: var(--accent); color: var(--accent-ink); }
    .anchor { position: relative; flex: none; }
    .popup { position: absolute; z-index: 20; top: calc(100% + 4px); left: 0; min-width: 260px; max-width: 520px; max-height: 240px; overflow: auto; margin: 0; padding: 4px 0; list-style: none; background: var(--bg-3); border: 1px solid var(--border-3); border-radius: 6px; box-shadow: var(--shadow); }
    .popup li { display: flex; gap: 10px; align-items: baseline; padding: 4px 10px; cursor: pointer; }
    .popup li.active, .popup li:hover { background: var(--bg-sel); }
    .kind { width: 58px; flex: none; font-size: var(--fs-2xs); color: var(--muted); }
    .history { top: 28px; right: 0; left: auto; width: 480px; }
    .history li { padding: 0; }
    .history .item { all: unset; box-sizing: border-box; width: 100%; min-height: 24px; padding: 4px 10px; cursor: pointer; font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .history .item:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .history .clear { border-top: 1px solid var(--border); font-family: var(--font-sans); color: var(--muted); }
  `,
})
export class QueryField {
  protected readonly field = inject(SearchField);
  protected readonly session = this.field.session;
  protected readonly store = inject(WorkspaceStore);
  protected readonly smart = inject(SmartSearchStore);
  protected readonly ai = inject(AiLauncher);
  protected readonly aiStatus = inject(AiStatusStore);
  private readonly saved = inject(SavedQueryStore);
  private readonly commands = inject(CommandRegistry);
  private readonly input = viewChild<ElementRef<HTMLTextAreaElement>>('input');

  protected readonly q = this.store.query;
  protected readonly runKey = formatKeybinding('Mod+Enter');
  protected readonly runTitle = $localize`:@@analysis.queryField.runTitle:Run the query (Enter or ${this.runKey}:key:)`;
  protected readonly focused = signal(false);
  protected readonly dismissed = signal(false);
  protected readonly historyOpen = signal(false);
  private readonly scrollX = signal(0);
  private readonly scrollY = signal(0);
  /** The highlighted completion (CDK ActiveDescendantKeyManager; focus stays in the field). */
  private readonly nav = new ActiveDescendantNavigation(computed(() => this.session.completions().length), inject(Injector), { wrap: true, selectFirst: true });
  protected readonly active = this.nav.active;

  protected readonly placeholder = computed(() =>
    this.q().regex ? $localize`:@@workspace.filterBar.regexPlaceholder:Regular expression` : $localize`:@@analysis.queryField.placeholder:Search or query…`,
  );
  protected readonly expanded = computed(() => this.focused() && (this.field.text().includes('\n') || this.lines() > 1));
  /** Lines the text takes at the field's width (measured on input). */
  private readonly lines = signal(1);
  protected readonly segments = computed(() => highlight(this.field.text(), this.session.error()));
  protected readonly shift = computed(() => `translate(${-this.scrollX()}px, ${-this.scrollY()}px)`);
  protected readonly caretError = computed(() => {
    const span = errorSpan(this.field.text(), this.session.error());
    return !!span && span.start === span.end;
  });
  protected readonly regexInvalid = computed(() => {
    const q = this.q();
    if (!q.regex || !q.text) return false;
    try {
      new RegExp(q.text.replace(/\(\?P</g, '(?<'));
      return false;
    } catch {
      return true;
    }
  });
  protected readonly invalid = computed(() => (this.field.kind() === 'query' && !!this.session.error()) || (this.field.kind() === 'text' && this.regexInvalid()));
  protected readonly describedBy = computed(() => (this.field.kind() === 'query' && (this.session.error() || this.session.runError()) ? 'ql-error' : null));
  protected readonly popupOpen = computed(() => this.focused() && !this.dismissed() && this.field.kind() === 'query' && this.session.completions().length > 0);
  protected readonly partialNote = computed(() => {
    const n = this.smart.added();
    return plural(n, {
      one: $localize`:@@workspace.filterBar.partialOne:${n}:count: more partial match added: the text appears inside a word`,
      other: $localize`:@@workspace.filterBar.partialMany:${n}:count: more partial matches added: the text appears inside words`,
    });
  });

  constructor() {
    this.field.start();
    // Ctrl+/ (focus the query) and Ctrl+F (Find) both land here; the commands may run before the bar is shown.
    effect(() => {
      this.session.focusNonce();
      const el = this.input();
      if (el && untracked(() => this.session.takeFocusRequest())) untracked(() => focusSoon(el.nativeElement));
    });
    effect(() => {
      if (this.store.focusSearchNonce() === 0) return;
      const el = this.input();
      untracked(() => setTimeout(() => el?.nativeElement.focus()));
    });
    // Text set from elsewhere (history, undo, a saved query) resizes the field too.
    effect(() => {
      this.field.text();
      untracked(() => queueMicrotask(() => this.resize()));
    });
  }

  private el(): HTMLTextAreaElement {
    return this.input()!.nativeElement;
  }

  protected onFocus(): void {
    this.focused.set(true);
    queueMicrotask(() => this.resize());
  }

  protected onBlur(): void {
    this.focused.set(false);
    queueMicrotask(() => this.resize());
  }

  protected onInput(): void {
    this.dismissed.set(false);
    this.nav.setActive(0);
    const el = this.el();
    void this.field.edit(el.value, charToByte(el.value, el.selectionStart ?? el.value.length));
    this.resize();
    this.sync();
  }

  /** Grows the field while focused (up to `MAX_FIELD_LINES` lines), one line otherwise. */
  private resize(): void {
    const el = this.input()?.nativeElement;
    if (!el) return;
    el.style.height = 'auto';
    const lines = Math.max(1, Math.round((el.scrollHeight - 8) / LINE_PX));
    this.lines.set(lines);
    const shown = this.focused() ? Math.min(lines, MAX_FIELD_LINES) : 1;
    el.style.height = `${shown * LINE_PX + 8}px`;
    this.sync();
  }

  /** Keeps the highlighted mirror aligned with the field's scroll. */
  protected sync(): void {
    const el = this.input()?.nativeElement;
    if (!el) return;
    this.scrollX.set(el.scrollLeft);
    this.scrollY.set(el.scrollTop);
  }

  protected onKey(e: KeyboardEvent): void {
    const completions = this.session.completions();
    const open = this.popupOpen();
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      e.stopPropagation();
      void this.submit();
    } else if (e.key === ' ' && e.ctrlKey) {
      e.preventDefault();
      this.dismissed.set(false);
      void this.session.complete(this.cursorBytes());
    } else if (open && (e.key === 'ArrowDown' || e.key === 'ArrowUp') && !e.altKey && this.nav.handle(e)) {
      e.preventDefault();
    } else if (open && (e.key === 'Enter' || e.key === 'Tab') && !e.shiftKey) {
      e.preventDefault();
      const picked = completions[Math.min(this.active(), completions.length - 1)];
      if (picked) this.accept(picked);
    } else if (e.key === 'Escape' && open) {
      e.preventDefault();
      e.stopPropagation();
      this.dismissed.set(true);
    } else if (e.key === 'Escape' && this.session.running()) {
      e.preventDefault();
      e.stopPropagation();
      void this.session.cancel();
    } else if (e.key === 'Escape' && this.field.text()) {
      e.preventDefault();
      e.stopPropagation();
      this.field.clear();
    } else if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      e.preventDefault();
      this.step(e.key === 'ArrowUp' ? 1 : -1);
    } else if (e.key === 'Enter' && !e.shiftKey) {
      // Shift+Enter is a new line; Enter runs a query (a message search already applies).
      e.preventDefault();
      void this.submit();
    }
  }

  protected accept(c: Completion): void {
    const el = this.el();
    const next = applyCompletion(el.value, el.selectionStart ?? el.value.length, c.insert);
    el.value = next.text;
    el.setSelectionRange(next.cursor, next.cursor);
    this.dismissed.set(false);
    this.nav.setActive(0);
    void this.field.edit(next.text, charToByte(next.text, next.cursor));
    this.sync();
    el.focus();
  }

  /** Replaces the unknown field the engine reported with the chosen suggestion (from the error line). */
  useSuggestion(name: string): void {
    const error = this.session.error();
    if (!error) return;
    const el = this.el();
    const next = applySuggestion(el.value, error, name);
    el.value = next.text;
    this.session.error.set(null);
    void this.field.edit(next.text, charToByte(next.text, next.cursor));
    el.focus();
    el.setSelectionRange(next.cursor, next.cursor);
    this.dismissed.set(true);
    this.sync();
  }

  protected useHistory(text: string): void {
    this.historyOpen.set(false);
    this.dismissed.set(true);
    void this.field.edit(text, new TextEncoder().encode(text).length);
    this.el().focus();
  }

  protected closePopups(): void {
    this.dismissed.set(true);
  }

  protected submit(): Promise<unknown> {
    this.dismissed.set(true);
    return this.field.submit();
  }

  /** Saves the search and filters as a saved query ("Query 3"); the toast opens the list to rename it. */
  protected save(): void {
    this.saved.saveCurrent('');
    const name = this.saved.items().at(-1)?.name ?? '';
    this.store.setNotice($localize`:@@analysis.queryField.saved:Saved as "${name}:name:"`, {
      label: $localize`:@@analysis.queryField.savedShow:Show saved queries`,
      run: () => void this.commands.run('savedQueries.show', 'button'),
    });
  }

  /** Alt+Up / Alt+Down walk through the history. */
  private step(delta: 1 | -1): void {
    const list = this.session.history.entries();
    if (!list.length) return;
    const at = list.indexOf(this.field.text().trim());
    const next = at < 0 ? (delta === 1 ? 0 : -1) : at + delta;
    const entry = list[next];
    if (next >= 0 && entry !== undefined) this.useHistory(entry);
  }

  private cursorBytes(): number {
    const el = this.el();
    return charToByte(el.value, el.selectionStart ?? el.value.length);
  }
}

/**
 * The line under the filter bar while the search field holds a query: its syntax error (with did-you-mean), the
 * run in flight, a cancelled or failed run, or what the applied query matched. Renders nothing otherwise.
 */
@Component({
  selector: 'debrief-query-status',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [QlSuggestions],
  host: { '[class.empty]': '!shown()' },
  template: `
    @if (shown()) {
      <div class="status" role="status" data-testid="query-status">
        @if (session.running()) {
          <span class="muted running">{{ session.cancelling() ? cancellingText : runningText }}
            @if (progress(); as t) {
              <progress [value]="session.progress()!.done" [max]="session.progress()!.total" aria-label="Query progress" i18n-aria-label="@@analysis.queryView.progress"></progress> {{ t }}
            }
          </span>
        } @else if (errorText(); as msg) {
          <span id="ql-error" class="err" role="alert"><span aria-hidden="true">⚠ </span>{{ msg }}</span>
          <debrief-ql-suggestions [error]="session.error()" (pick)="pick.emit($event)" />
        } @else if (session.runError(); as r) {
          <span id="ql-error" class="err">⚠ {{ r }}</span>
        } @else if (session.cancelled()) {
          <span class="muted" data-testid="query-cancelled" i18n="@@analysis.queryView.cancelled">Query cancelled.</span>
        } @else if (session.applied(); as ql) {
          <span class="applied" data-testid="query-applied"><ng-container i18n="@@analysis.queryView.filtering">Filtering with <code class="mono">{{ ql }}</code></ng-container>@if (session.matched() !== null) { <ng-container i18n="@@analysis.queryView.matched"> · {{ session.matched() }} events </ng-container> }
            <button class="link" type="button" (click)="field.clear()" i18n="Remove the applied query|Button@@analysis.queryView.clear">Clear</button></span>
        }
      </div>
    }
  `,
  styles: `
    :host { display: block; }
    :host(.empty) { display: none; }
    .status { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; min-height: 18px; font-size: var(--fs-sm); color: var(--text-2); }
    .err { color: var(--danger-text); overflow-wrap: anywhere; }
    .muted { color: var(--muted); }
    .applied code { color: var(--text); }
    .running { display: inline-flex; gap: 8px; align-items: center; }
    .running progress { width: 140px; height: 6px; accent-color: var(--accent); }
    .link { background: none; border: 0; padding: 0 0 0 6px; font: inherit; color: var(--chip-text); cursor: pointer; text-decoration: underline; }
  `,
})
export class QueryStatus {
  protected readonly field = inject(SearchField);
  protected readonly session = this.field.session;
  readonly pick = output<string>();

  protected readonly cancellingText = $localize`:@@analysis.queryView.cancelling:Cancelling…`;
  protected readonly runningText = $localize`:@@analysis.queryView.running:Running…`;
  protected readonly progress = computed(() => progressText(this.session.progress()));
  /** The engine's syntax error as a sentence with its column ("Expected a value at column 10"), or null. */
  protected readonly errorText = computed(() => {
    const e = this.session.error();
    if (!e || this.field.kind() !== 'query') return null;
    const message = e.message.charAt(0).toUpperCase() + e.message.slice(1);
    const column = (errorSpan(this.session.text(), e)?.start ?? 0) + 1;
    return e.len || e.offset ? $localize`:@@analysis.queryView.errorAt:${message}:message: at column ${column}:column:` : message;
  });
  protected readonly shown = computed(
    () => this.field.kind() === 'query' && (this.session.running() || !!this.errorText() || !!this.session.runError() || this.session.cancelled() || !!this.session.applied()),
  );
}
