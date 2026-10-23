import { formatCount } from './format/units';
import { ChangeDetectionStrategy, Component, ElementRef, inject, input, output } from '@angular/core';

/** One entry of a {@link Tabs} strip. */
export interface TabItem {
  id: string;
  label: string;
  /** A muted count after the label; hidden when 0 or unset. */
  count?: number;
}

/**
 * The one tab strip of Debrief (Plugins, Store, Jobs, and the same underline the dock's tabs wear): text tabs with an accent
 * underline under the selected one, 32 px high, one tab stop with arrow/Home/End keys. `kind="tabs"` is the ARIA tab
 * pattern (the host owns the panel and points its `aria-controls` at it through `panelId`); `kind="filter"` is a group of
 * toggle buttons (`aria-pressed`) for a list filter that has no panel of its own.
 */
@Component({
  selector: 'debrief-tabs',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[attr.role]': 'kind() === "tabs" ? "tablist" : "group"', '[attr.aria-label]': 'label()' },
  template: `
    @for (t of items(); track t.id) {
      <button
        type="button"
        class="tab"
        [attr.role]="kind() === 'tabs' ? 'tab' : null"
        [attr.id]="kind() === 'tabs' ? idPrefix() + t.id : null"
        [attr.data-testid]="idPrefix() + t.id"
        [attr.aria-selected]="kind() === 'tabs' ? selected() === t.id : null"
        [attr.aria-pressed]="kind() === 'filter' ? selected() === t.id : null"
        [attr.aria-controls]="kind() === 'tabs' && panelId() ? panelId()!(t.id) : null"
        [attr.tabindex]="kind() === 'tabs' && selected() !== t.id ? -1 : null"
        [class.on]="selected() === t.id"
        (click)="pick(t.id)"
        (keydown)="onKey($event, t.id)"
      >{{ t.label }}@if (t.count) { <span class="count">{{ fmt(t.count) }}</span> }</button>
    }
  `,
  styles: `
    :host { display: flex; gap: 2px; min-width: 0; }
    .tab { height: 32px; padding: 0 10px; border: 0; border-bottom: 2px solid transparent; background: transparent; color: var(--muted); font: inherit; font-size: var(--fs-md); cursor: pointer; white-space: nowrap; display: inline-flex; align-items: center; gap: 6px; }
    .tab:hover { color: var(--text-2); }
    .tab.on { color: var(--text); border-bottom-color: var(--accent); }
    .tab:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; border-radius: 4px; }
    .count { font-size: var(--fs-xs); font-variant-numeric: tabular-nums; color: var(--muted); }
  `,
})
export class Tabs {
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
  protected readonly fmt = formatCount;
  readonly items = input.required<readonly TabItem[]>();
  readonly selected = input.required<string>();
  /** The accessible name of the strip. */
  readonly label = input.required<string>();
  /** Prefix of every tab's DOM id and `data-testid` (`plugins-tab-` gives `plugins-tab-installed`). */
  readonly idPrefix = input.required<string>();
  readonly kind = input<'tabs' | 'filter'>('tabs');
  /** The id of the panel a tab controls (`kind="tabs"`). */
  readonly panelId = input<((id: string) => string) | null>(null);
  readonly selectedChange = output<string>();

  protected pick(id: string): void {
    if (id !== this.selected()) this.selectedChange.emit(id);
  }

  /** Arrow keys, Home and End move between the tabs and select the one reached (tabs), or only focus it (filter). */
  protected onKey(e: KeyboardEvent, from: string): void {
    const ids = this.items().map((t) => t.id);
    const i = ids.indexOf(from);
    const to = e.key === 'ArrowRight' ? i + 1 : e.key === 'ArrowLeft' ? i - 1 : e.key === 'Home' ? 0 : e.key === 'End' ? ids.length - 1 : null;
    if (to === null || !ids.length) return;
    e.preventDefault();
    const id = ids[(to + ids.length) % ids.length]!;
    if (this.kind() === 'tabs') this.selectedChange.emit(id);
    queueMicrotask(() => this.host.querySelector<HTMLElement>(`[data-testid="${this.idPrefix()}${id}"]`)?.focus());
  }
}
