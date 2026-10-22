import { CdkListbox, CdkOption, ListboxValueChangeEvent } from '@angular/cdk/listbox';
import { CdkConnectedOverlay, CdkOverlayOrigin, ConnectedPosition } from '@angular/cdk/overlay';
import { ChangeDetectionStrategy, Component, computed, input, output, signal, viewChild } from '@angular/core';
import { formatCount } from '../shared/format/units';

/** One choice of a `KindFilter`: what it is called and how many items it would show. */
export interface KindOption {
  id: string;
  label: string;
  count: number;
}

/**
 * A "Kind ▾" filter: one button that says what is filtered ("Kind" or "Kind: 2") and opens a multi-select CDK listbox
 * (keyboard, roles and focus from the CDK) with counts, in a CDK overlay. Used by Jobs and Notifications instead of
 * a row of chips that look like buttons (K5). Escape or a click outside closes it.
 */
@Component({
  selector: 'imt-kind-filter',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CdkListbox, CdkOption, CdkConnectedOverlay, CdkOverlayOrigin],
  template: `
    <button type="button" class="btn small trigger" data-testid="kind-filter" cdkOverlayOrigin #origin="cdkOverlayOrigin" aria-haspopup="listbox" [attr.aria-expanded]="open()" (click)="open.set(!open())">
      {{ summary() }} <span aria-hidden="true">▾</span>
    </button>
    <ng-template
      cdkConnectedOverlay
      [cdkConnectedOverlayOrigin]="origin"
      [cdkConnectedOverlayOpen]="open()"
      [cdkConnectedOverlayPositions]="positions"
      (overlayOutsideClick)="outside($event, origin.elementRef.nativeElement)"
      (attach)="focusList()"
      (detach)="open.set(false)"
    >
      <div class="list" data-testid="kind-filter-list" (keydown.escape)="open.set(false)">
        <ul cdkListbox cdkListboxMultiple class="options" [attr.aria-label]="label()" [cdkListboxValue]="selected()" (cdkListboxValueChange)="changed($event)">
          @for (o of options(); track o.id) {
            <li class="opt" [cdkOption]="o.id" [attr.data-testid]="'kind-' + o.id">
              <span class="check" aria-hidden="true">{{ selected().includes(o.id) ? '✓' : '' }}</span>
              <span class="name">{{ o.label }}</span>
              <span class="count muted">{{ fmt(o.count) }}</span>
            </li>
          }
        </ul>
        @if (selected().length) {
          <button type="button" class="btn ghost small" data-testid="kind-filter-clear" (click)="cleared.emit()" i18n="@@ui.kindFilter.clear">Show all</button>
        }
      </div>
    </ng-template>
  `,
  styles: `
    :host { display: inline-block; }
    .list { margin: 2px 0 0; padding: 6px 8px; min-width: 190px; display: flex; flex-direction: column; gap: 2px; border: 1px solid var(--border-3); border-radius: var(--r-2); background: var(--bg-3); box-shadow: var(--shadow-pop); }
    .options { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; outline: none; }
    .opt { display: flex; align-items: center; gap: 8px; min-height: 24px; padding: 0 4px; border-radius: 4px; color: var(--text-2); cursor: pointer; }
    .opt:hover, .opt.cdk-option-active { background: var(--bg-hover); }
    .opt[aria-selected='true'] { color: var(--text); }
    .opt .check { inline-size: 14px; color: var(--accent, currentColor); }
    .opt .name { flex: 1; }
    .count { font-size: var(--fs-xs); }
    .trigger:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
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

  protected readonly fmt = formatCount;
  private readonly listbox = viewChild(CdkListbox);
  protected readonly open = signal(false);
  protected readonly positions: ConnectedPosition[] = [
    { originX: 'start', originY: 'bottom', overlayX: 'start', overlayY: 'top' },
    { originX: 'start', originY: 'top', overlayX: 'start', overlayY: 'bottom' },
  ];
  protected readonly summary = computed(() => (this.selected().length ? `${this.label()}: ${this.selected().length}` : this.label()));

  /** The keyboard works as soon as the list opens. */
  protected focusList(): void {
    queueMicrotask(() => this.listbox()?.focus());
  }

  /** The listbox reports the whole selection; the filter reports what flipped. */
  protected changed(e: ListboxValueChangeEvent<string>): void {
    const before = new Set(this.selected());
    const after = new Set(e.value);
    for (const id of new Set([...before, ...after])) {
      if (before.has(id) !== after.has(id)) this.toggled.emit(id);
    }
  }

  /** A click on the trigger is its own toggle; any other click outside closes the list. */
  protected outside(e: MouseEvent, trigger: HTMLElement): void {
    if (!(e.target instanceof Node && trigger.contains(e.target))) this.open.set(false);
  }
}
