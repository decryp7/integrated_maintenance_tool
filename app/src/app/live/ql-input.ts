import { ChangeDetectionStrategy, Component, computed, DestroyRef, ElementRef, inject, input, output, signal, viewChild } from '@angular/core';
import { Completion, QlError } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { applyCompletion, applySuggestion, charToByte } from '../analysis/ql-text';
import { QlSuggestions } from '../analysis/ql-suggestions';

/** Delay before the engine checks the text and suggests completions. */
export const QL_CHECK_MS = 150;

let nextId = 1;

/**
 * A one-line query-language input with the engine's completions and syntax check (`complete_query`,
 * `check_query`), for conditions such as `[[live.rule]] when`. Keyboard: ↑/↓ choose, Enter/Tab accept,
 * Ctrl+Space suggests, Escape closes.
 */
@Component({
  selector: 'imt-ql-input',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [QlSuggestions],
  template: `
    <div class="wrap">
      <input
        #input
        class="input mono"
        type="text"
        role="combobox"
        aria-autocomplete="list"
        autocomplete="off"
        spellcheck="false"
        [attr.aria-label]="label()"
        [attr.aria-expanded]="open()"
        [attr.aria-controls]="open() ? listId : null"
        [attr.aria-activedescendant]="open() ? listId + '-' + active() : null"
        [attr.aria-invalid]="error() ? 'true' : null"
        [attr.aria-describedby]="error() ? errorId : null"
        [value]="value()"
        (input)="onInput()"
        (keydown)="onKey($event)"
        (blur)="onBlur()"
      />
      @if (open()) {
        <ul class="popup" [id]="listId" role="listbox" [attr.aria-label]="label() + ' completions'">
          @for (c of completions(); track c.label + $index; let i = $index) {
            <li [id]="listId + '-' + i" role="option" [attr.aria-selected]="i === active()" [class.active]="i === active()" (mousedown)="$event.preventDefault()" (click)="accept(c)">
              <span class="kind">{{ c.kind }}</span><span class="mono">{{ c.label }}</span>
            </li>
          }
        </ul>
      }
    </div>
    @if (error(); as e) { <div class="err" [id]="errorId">{{ e.message }} <imt-ql-suggestions [error]="e" (pick)="useSuggestion($event)" /></div> }
  `,
  styles: `
    :host { display: block; position: relative; min-width: 0; }
    .wrap { position: relative; }
    .input { width: 100%; box-sizing: border-box; }
    .popup { position: absolute; z-index: 20; left: 0; right: 0; top: calc(100% + 2px); margin: 0; padding: 4px 0; list-style: none; max-height: 200px; overflow: auto; background: var(--bg-3); border: 1px solid var(--border-2); border-radius: 6px; box-shadow: var(--shadow); }
    .popup li { display: flex; gap: 8px; padding: 3px 10px; cursor: pointer; font-size: 12px; }
    .popup li.active, .popup li:hover { background: var(--bg-active); }
    .kind { color: var(--muted); width: 64px; flex: none; font-size: 11px; }
    .err { margin-top: 3px; font-size: 11px; color: var(--danger-text); }
  `,
})
export class QlInput {
  readonly value = input('');
  readonly label = input('Condition');
  readonly valueChange = output<string>();

  private readonly backend = inject(AnalysisBackend);
  private readonly el = viewChild.required<ElementRef<HTMLInputElement>>('input');
  protected readonly completions = signal<Completion[]>([]);
  protected readonly error = signal<QlError | null>(null);
  protected readonly active = signal(0);
  private readonly dismissed = signal(true);
  protected readonly open = computed(() => !this.dismissed() && this.completions().length > 0);
  protected readonly listId = `ql-input-${nextId++}`;
  protected readonly errorId = `${this.listId}-err`;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private seq = 0;

  constructor() {
    inject(DestroyRef).onDestroy(() => clearTimeout(this.timer));
  }

  protected onInput(): void {
    const el = this.el().nativeElement;
    this.valueChange.emit(el.value);
    this.dismissed.set(false);
    this.schedule();
  }

  protected onBlur(): void {
    this.dismissed.set(true);
  }

  protected onKey(e: KeyboardEvent): void {
    const list = this.completions();
    const open = this.open();
    if (e.key === ' ' && e.ctrlKey) {
      e.preventDefault();
      this.dismissed.set(false);
      void this.check();
    } else if (open && e.key === 'ArrowDown') {
      e.preventDefault();
      this.active.set((this.active() + 1) % list.length);
    } else if (open && e.key === 'ArrowUp') {
      e.preventDefault();
      this.active.set((this.active() - 1 + list.length) % list.length);
    } else if (open && (e.key === 'Enter' || e.key === 'Tab')) {
      e.preventDefault();
      this.accept(list[Math.min(this.active(), list.length - 1)]);
    } else if (open && e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      this.dismissed.set(true);
    }
  }

  protected accept(c: Completion): void {
    const el = this.el().nativeElement;
    const next = applyCompletion(el.value, el.selectionStart ?? el.value.length, c.insert);
    el.value = next.text;
    el.setSelectionRange(next.cursor, next.cursor);
    this.active.set(0);
    this.valueChange.emit(next.text);
    this.schedule();
  }

  /** Replaces the unknown field the engine reported with the chosen suggestion. */
  protected useSuggestion(name: string): void {
    const error = this.error();
    if (!error) return;
    const el = this.el().nativeElement;
    const next = applySuggestion(el.value, error, name);
    el.value = next.text;
    el.focus();
    el.setSelectionRange(next.cursor, next.cursor);
    this.error.set(null);
    this.valueChange.emit(next.text);
    this.schedule();
  }

  private schedule(): void {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => void this.check(), QL_CHECK_MS);
  }

  /** Asks the engine for the syntax error and the completions at the caret. */
  async check(): Promise<void> {
    const el = this.el().nativeElement;
    const text = el.value;
    const cursor = charToByte(text, el.selectionStart ?? text.length);
    const mine = ++this.seq;
    try {
      const [error, completions] = await Promise.all([text.trim() ? this.backend.checkQuery(text) : Promise.resolve(null), this.backend.completeQuery(text, cursor)]);
      if (mine !== this.seq) return;
      this.error.set(error);
      this.completions.set(completions.slice(0, 50));
      this.active.set(0);
    } catch {
      if (mine === this.seq) this.completions.set([]);
    }
  }
}
