import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, signal, untracked, viewChild } from '@angular/core';
import { ClickOutside } from '../shared/click-outside';
import { Completion } from '../core/api.types';
import { formatKeybinding } from '../extensions/keybinding';
import { applyCompletion, applySuggestion, charToByte, errorSpan, highlight } from './ql-text';
import { QlSuggestions } from './ql-suggestions';
import { QuerySession } from './query-session';
import { AiButton } from '../ai/ai-button';
import { AiLauncher } from '../ai/ai-launcher';

/** Focuses the element, retrying while its tab is still being shown (the panel may be hidden or not yet laid out). */
export function focusSoon(el: HTMLElement, attempts = 20): void {
  el.focus();
  if (document.activeElement !== el && attempts > 0) setTimeout(() => focusSoon(el, attempts - 1), 50);
}

/**
 * The query bar: a monospace input with syntax highlighting (a transparent input over a
 * highlighted mirror), a red squiggle at the span the engine reports, a completions popup and history.
 */
@Component({
  selector: 'imt-query-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ClickOutside, QlSuggestions, AiButton],
  template: `
    <div class="qbar">
      <div class="editor" [class.invalid]="!!session.error()" (imtClickOutside)="closePopups()">
        <div class="mirror mono" aria-hidden="true"><span class="shift" [style.transform]="'translateX(' + -scroll() + 'px)'">@for (s of segments(); track $index) {<span [class]="s.cls" [class.err]="s.err">{{ s.text }}</span>}@if (caretError()) {<span class="err-caret"></span>}</span></div>
        <input
          #input
          class="qinput mono"
          type="text"
          role="combobox"
          aria-label="Query"
          aria-autocomplete="list"
          aria-haspopup="listbox"
          autocomplete="off"
          spellcheck="false"
          placeholder='level >= warning and message contains "timeout" | stats count() by source'
          [attr.aria-expanded]="popupOpen()"
          [attr.aria-controls]="popupOpen() ? 'ql-completions' : null"
          [attr.aria-activedescendant]="popupOpen() ? 'ql-opt-' + active() : null"
          [attr.aria-invalid]="session.error() ? 'true' : null"
          [attr.aria-describedby]="session.error() ? 'ql-error' : null"
          [value]="session.text()"
          (input)="onInput()"
          (keydown)="onKey($event)"
          (keyup)="sync()"
          (click)="sync()"
          (focus)="focused.set(true)"
          (blur)="focused.set(false)"
        />
        @if (popupOpen()) {
          <ul class="popup" id="ql-completions" role="listbox" aria-label="Completions">
            @for (c of session.completions(); track c.label + $index; let i = $index) {
              <li [id]="'ql-opt-' + i" role="option" [attr.aria-selected]="i === active()" [class.active]="i === active()" (mousedown)="$event.preventDefault()" (click)="accept(c)">
                <span class="kind">{{ c.kind }}</span><span class="mono">{{ c.label }}</span>
              </li>
            }
          </ul>
        }
      </div>
      <button class="btn primary" type="button" (click)="run()" [disabled]="session.running()" [title]="'Run (' + runKey + ')'">Run</button>
      <imt-ai-button label="Ask AI" hint="Describe what you want in plain words; the local model writes the query" testId="ai-ask-query" (pressed)="ai.askQuery()" />
      <div class="anchor" (imtClickOutside)="historyOpen.set(false)">
        <button class="btn" type="button" (click)="historyOpen.set(!historyOpen())" [attr.aria-expanded]="historyOpen()" aria-haspopup="true" [disabled]="!session.history.entries().length">History</button>
        @if (historyOpen()) {
          <ul class="popup history" aria-label="Query history">
            @for (h of session.history.entries(); track h) {
              <li><button class="item mono" type="button" (click)="useHistory(h)">{{ h }}</button></li>
            }
            <li class="clear"><button class="item" type="button" (click)="session.history.clear(); historyOpen.set(false)">Clear history</button></li>
          </ul>
        }
      </div>
    </div>
    <div class="status" role="status">
      @if (session.error(); as e) {
        <span id="ql-error" class="err-msg">⚠ {{ e.message }}@if (e.len || e.offset) { <span class="muted"> (at column {{ column() }})</span> }</span>
        <imt-ql-suggestions [error]="e" (pick)="useSuggestion($event)" />
      } @else if (session.runError(); as r) {
        <span class="err-msg">⚠ {{ r }}</span>
      } @else if (session.applied(); as ql) {
        <span class="applied">Filtering with <code class="mono">{{ ql }}</code>@if (session.matched() !== null) { · {{ session.matched() }} events }
          <button class="link" type="button" (click)="session.clearApplied()">Clear</button></span>
      } @else {
        <span class="muted">{{ runKey }} to run. Without <code class="mono">stats</code> the query filters the grid, timeline and charts; with it the table opens in Query results.</span>
      }
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; justify-content: flex-start; gap: 4px; padding: 8px 12px; background: var(--bg-1); color: var(--text-2); font-size: 12px; }
    .qbar { display: flex; gap: 8px; align-items: center; }
    .editor { position: relative; flex: 1; min-width: 0; height: 30px; border: 1px solid var(--border-3); border-radius: 5px; background: var(--bg-3); }
    .editor:focus-within { border-color: var(--accent); }
    .editor.invalid { border-color: var(--danger-text); }
    .mirror, .qinput { position: absolute; inset: 0; padding: 0 9px; font: 13px/28px var(--font-mono); white-space: pre; }
    .mirror { overflow: hidden; pointer-events: none; }
    .shift { display: inline-block; }
    .qinput { width: 100%; box-sizing: border-box; border: 0; outline: 0; background: transparent; color: transparent; caret-color: var(--text); }
    .qinput::placeholder { color: var(--faint); }
    .qinput::selection { background: var(--accent-soft); color: transparent; }
    .kw { color: var(--chip-text); font-weight: 600; }
    .fn { color: var(--ok); }
    .field { color: var(--accent); }
    .str { color: var(--warn-text); }
    .num { color: var(--ok); }
    .op, .punct { color: var(--muted); }
    .text { color: var(--text); }
    .err { text-decoration: underline wavy var(--danger-text); text-underline-offset: 3px; }
    .err-caret { display: inline-block; width: 8px; height: 14px; border-bottom: 2px solid var(--danger-text); vertical-align: bottom; }
    .popup { position: absolute; z-index: 20; top: 32px; left: 0; min-width: 260px; max-width: 520px; max-height: 240px; overflow: auto; margin: 0; padding: 4px 0; list-style: none; background: var(--bg-3); border: 1px solid var(--border-3); border-radius: 6px; box-shadow: var(--shadow); }
    .popup li { display: flex; gap: 10px; align-items: baseline; padding: 4px 10px; cursor: pointer; }
    .popup li.active, .popup li:hover { background: var(--bg-sel); }
    .kind { width: 58px; flex: none; font-size: 10px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
    .anchor { position: relative; }
    .history { right: 0; left: auto; width: 480px; }
    .history li { padding: 0; }
    .history .item { all: unset; box-sizing: border-box; width: 100%; padding: 4px 10px; cursor: pointer; font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .history .item:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
    .history .clear { border-top: 1px solid var(--border); font-family: var(--font-sans); color: var(--muted); }
    .status { min-height: 16px; font-size: 12px; }
    .err-msg { color: var(--danger-text); }
    .muted { color: var(--muted); }
    .applied code, .muted code { color: var(--text); }
    .link { background: none; border: 0; padding: 0 0 0 6px; font: inherit; color: var(--chip-text); cursor: pointer; text-decoration: underline; }
  `,
})
export class QueryView {
  protected readonly ai = inject(AiLauncher);
  protected readonly session = inject(QuerySession);
  private readonly input = viewChild<ElementRef<HTMLInputElement>>('input');

  private el(): HTMLInputElement {
    return this.input()!.nativeElement;
  }

  protected readonly runKey = formatKeybinding('Mod+Enter');
  protected readonly focused = signal(false);
  protected readonly dismissed = signal(false);
  protected readonly active = signal(0);
  protected readonly historyOpen = signal(false);
  protected readonly scroll = signal(0);

  protected readonly segments = computed(() => highlight(this.session.text(), this.session.error()));
  protected readonly caretError = computed(() => {
    const e = this.session.error();
    const span = errorSpan(this.session.text(), e);
    return !!span && span.start === span.end;
  });
  protected readonly column = computed(() => (errorSpan(this.session.text(), this.session.error())?.start ?? 0) + 1);
  protected readonly popupOpen = computed(() => this.focused() && !this.dismissed() && this.session.completions().length > 0);

  constructor() {
    // The "focus query bar" command may run before this view exists (it is created lazily).
    effect(() => {
      this.session.focusNonce();
      const el = this.input();
      if (el && untracked(() => this.session.takeFocusRequest())) untracked(() => focusSoon(el.nativeElement));
    });
  }

  protected onInput(): void {
    this.dismissed.set(false);
    this.active.set(0);
    const el = this.el();
    this.session.edit(el.value, charToByte(el.value, el.selectionStart ?? el.value.length));
    this.sync();
  }

  /** Keeps the highlighted mirror aligned with the input's horizontal scroll. */
  protected sync(): void {
    this.scroll.set(this.el().scrollLeft);
  }

  protected onKey(e: KeyboardEvent): void {
    const completions = this.session.completions();
    const open = this.popupOpen();
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      e.stopPropagation();
      void this.run();
    } else if (e.key === ' ' && e.ctrlKey) {
      e.preventDefault();
      this.dismissed.set(false);
      void this.session.complete(this.cursorBytes());
    } else if (open && e.key === 'ArrowDown') {
      e.preventDefault();
      this.active.set((this.active() + 1) % completions.length);
    } else if (open && e.key === 'ArrowUp') {
      e.preventDefault();
      this.active.set((this.active() - 1 + completions.length) % completions.length);
    } else if (open && (e.key === 'Enter' || e.key === 'Tab')) {
      e.preventDefault();
      this.accept(completions[Math.min(this.active(), completions.length - 1)]);
    } else if (e.key === 'Escape' && open) {
      e.preventDefault();
      e.stopPropagation();
      this.dismissed.set(true);
    } else if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      e.preventDefault();
      this.step(e.key === 'ArrowUp' ? 1 : -1);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      void this.run();
    }
  }

  protected accept(c: Completion): void {
    const el = this.el();
    const next = applyCompletion(el.value, el.selectionStart ?? el.value.length, c.insert);
    el.value = next.text;
    el.setSelectionRange(next.cursor, next.cursor);
    this.dismissed.set(false);
    this.active.set(0);
    this.session.edit(next.text, charToByte(next.text, next.cursor));
    this.sync();
    el.focus();
  }

  /** Replaces the unknown field the engine reported with the chosen suggestion. */
  protected useSuggestion(name: string): void {
    const error = this.session.error();
    if (!error) return;
    const el = this.el();
    const next = applySuggestion(el.value, error, name);
    el.value = next.text;
    this.session.error.set(null);
    this.session.edit(next.text, charToByte(next.text, next.cursor));
    el.focus();
    el.setSelectionRange(next.cursor, next.cursor);
    this.dismissed.set(true);
    this.sync();
  }

  protected useHistory(text: string): void {
    this.session.load(text);
    this.historyOpen.set(false);
    this.dismissed.set(true);
    this.el().focus();
  }

  protected closePopups(): void {
    this.dismissed.set(true);
  }

  protected run(): Promise<'events' | 'table' | null> {
    this.dismissed.set(true);
    return this.session.run();
  }

  /** Alt+Up / Alt+Down walk through the history. */
  private step(delta: 1 | -1): void {
    const list = this.session.history.entries();
    if (!list.length) return;
    const at = list.indexOf(this.session.text().trim());
    const next = at < 0 ? (delta === 1 ? 0 : -1) : at + delta;
    if (next >= 0 && next < list.length) this.useHistory(list[next]);
  }

  private cursorBytes(): number {
    const el = this.el();
    return charToByte(el.value, el.selectionStart ?? el.value.length);
  }
}
