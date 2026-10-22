import { ChangeDetectionStrategy, Component, computed, input, output, signal } from '@angular/core';

/** One choice of a `KindFilter`: what it is called and how many items it would show. */
export interface KindOption {
  id: string;
  label: string;
  count: number;
}

/**
 * A "Kind ▾" filter: one button that says what is filtered ("Kind" or "Kind: 2") and opens a list of checkboxes with
 * counts. Used by Jobs and Notifications instead of a row of chips that look like buttons (K5). Escape closes it.
 */
@Component({
  selector: 'imt-kind-filter',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '(keydown.escape)': 'close($event)' },
  template: `
    <button type="button" class="btn small trigger" data-testid="kind-filter" aria-haspopup="true" [attr.aria-expanded]="open()" [attr.aria-controls]="id" (click)="open.set(!open())">
      {{ summary() }} <span aria-hidden="true">▾</span>
    </button>
    @if (open()) {
      <fieldset class="list" [id]="id" data-testid="kind-filter-list">
        <legend class="sr-only">{{ label() }}</legend>
        @for (o of options(); track o.id) {
          <label class="opt">
            <input type="checkbox" [checked]="selected().includes(o.id)" (change)="toggled.emit(o.id)" [attr.data-testid]="'kind-' + o.id" />
            <span class="name">{{ o.label }}</span>
            <span class="count muted">{{ o.count }}</span>
          </label>
        }
        @if (selected().length) {
          <button type="button" class="btn ghost small" data-testid="kind-filter-clear" (click)="cleared.emit()" i18n="@@ui.kindFilter.clear">Show all</button>
        }
      </fieldset>
    }
  `,
  styles: `
    :host { position: relative; display: inline-block; }
    .list { position: absolute; inset-block-start: 100%; inset-inline-start: 0; z-index: 20; margin: 2px 0 0; padding: 6px 8px; min-width: 190px; display: flex; flex-direction: column; gap: 2px; border: 1px solid var(--border-3); border-radius: var(--r-2); background: var(--bg-3); box-shadow: var(--shadow-pop); }
    .opt { display: flex; align-items: center; gap: 8px; min-height: 24px; color: var(--text-2); cursor: pointer; }
    .opt .name { flex: 1; }
    .count { font-size: var(--fs-xs); }
    .trigger:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  `,
})
export class KindFilter {
  /** What the filter is about ("Kind", "Source"). */
  readonly label = input.required<string>();
  readonly options = input.required<readonly KindOption[]>();
  /** The ids that are on; empty shows everything. */
  readonly selected = input.required<readonly string[]>();
  readonly toggled = output<string>();
  readonly cleared = output<void>();

  protected readonly open = signal(false);
  protected readonly id = `imt-kind-filter-${++seq}`;
  protected readonly summary = computed(() => (this.selected().length ? `${this.label()}: ${this.selected().length}` : this.label()));

  protected close(e: Event): void {
    if (!this.open()) return;
    e.stopPropagation();
    this.open.set(false);
  }
}

let seq = 0;
