import { ChangeDetectionStrategy, Component, computed, inject, signal, ViewEncapsulation } from '@angular/core';
import { CommandRegistry } from '../extensions/command-registry';
import { FIXED_KEYS, shortcutGroups } from './shortcuts';

/** Class on `<body>` while the cheat sheet is printed (the print styles below show only the sheet). */
export const PRINT_CLASS = 'imt-print-cheat-sheet';

/**
 * The keyboard cheat sheet (F1): every command's key binding by category plus the keys components handle
 * themselves, searchable and printable. Generated, so it always matches what is registered.
 */
@Component({
  selector: 'imt-cheat-sheet',
  changeDetection: ChangeDetectionStrategy.OnPush,
  // Not encapsulated: the print rules must hide the rest of the app. Every rule is scoped to the sheet or the print class.
  encapsulation: ViewEncapsulation.None,
  template: `
    <div class="cs-bar">
      <input class="input cs-search" type="search" aria-label="Search shortcuts" i18n-aria-label="@@help.cheatSheet.search.label" placeholder="Search shortcuts…" i18n-placeholder="@@help.cheatSheet.search.placeholder" [value]="query()" (input)="query.set($any($event.target).value)" />
      <button type="button" class="btn" (click)="print()" i18n="@@help.cheatSheet.print">Print…</button>
    </div>
    <p class="cs-count" role="status" i18n="@@help.cheatSheet.count">{count(), plural, =1 {{{ count() }} shortcut} other {{{ count() }} shortcuts}}</p>
    @for (g of groups(); track g.title) {
      <section class="cs-group" [attr.aria-label]="g.title">
        <h3>{{ g.title }}</h3>
        <dl>
          @for (s of g.items; track s.description + s.keys) {
            <div class="cs-row"><dt><kbd>{{ s.keys }}</kbd></dt><dd>{{ s.description }}</dd></div>
          }
        </dl>
      </section>
    } @empty {
      <p class="cs-none" i18n="@@help.cheatSheet.none">No shortcut matches “{{ query() }}”. <button type="button" class="btn ghost" (click)="query.set('')">Clear search</button></p>
    }
  `,
  styles: `
    imt-cheat-sheet { display: flex; flex-direction: column; gap: 10px; }
    imt-cheat-sheet .cs-bar { display: flex; gap: 8px; }
    imt-cheat-sheet .cs-search { flex: 1; }
    imt-cheat-sheet .cs-count { margin: 0; font-size: var(--fs-xs); color: var(--muted); }
    imt-cheat-sheet .cs-group h3 { margin: 6px 0 4px; font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); font-weight: 600; }
    imt-cheat-sheet dl { margin: 0; display: grid; gap: 2px; }
    imt-cheat-sheet .cs-row { display: grid; grid-template-columns: 180px 1fr; gap: 12px; align-items: baseline; padding: 3px 0; border-bottom: 1px solid var(--border); }
    imt-cheat-sheet dt, imt-cheat-sheet dd { margin: 0; }
    imt-cheat-sheet dd { color: var(--text-2); }
    imt-cheat-sheet kbd { font-family: var(--font-mono); font-size: var(--fs-sm); padding: 1px 6px; border-radius: 4px; border: 1px solid var(--border-3); background: var(--bg-3); color: var(--text); }
    imt-cheat-sheet .cs-none { color: var(--muted); }
    @media print {
      body.${PRINT_CLASS} * { visibility: hidden !important; }
      body.${PRINT_CLASS} imt-cheat-sheet, body.${PRINT_CLASS} imt-cheat-sheet * { visibility: visible !important; }
      body.${PRINT_CLASS} imt-cheat-sheet { position: absolute; inset: 0 auto auto 0; width: 100%; color: #000; background: #fff; }
      body.${PRINT_CLASS} imt-cheat-sheet .cs-bar { display: none; }
      body.${PRINT_CLASS} imt-cheat-sheet kbd { color: #000; background: #fff; border-color: #888; }
      body.${PRINT_CLASS} imt-cheat-sheet dd, body.${PRINT_CLASS} imt-cheat-sheet .cs-group h3 { color: #000; }
      body.${PRINT_CLASS} .dialog, body.${PRINT_CLASS} .dialog .body { position: static !important; max-height: none !important; overflow: visible !important; transform: none !important; }
    }
  `,
})
export class CheatSheet {
  private readonly registry = inject(CommandRegistry);
  protected readonly query = signal('');
  protected readonly groups = computed(() => shortcutGroups(this.registry.all(), FIXED_KEYS, this.query()));
  protected readonly count = computed(() => this.groups().reduce((n, g) => n + g.items.length, 0));

  /** Prints only the sheet (the rest of the app is hidden by the print styles while the class is set). */
  protected print(): void {
    const body = document.body;
    body.classList.add(PRINT_CLASS);
    const done = () => {
      body.classList.remove(PRINT_CLASS);
      window.removeEventListener('afterprint', done);
    };
    window.addEventListener('afterprint', done);
    try {
      window.print();
    } finally {
      // Browsers that print asynchronously still fire afterprint; the timeout covers those that do not.
      setTimeout(done, 1000);
    }
  }
}
