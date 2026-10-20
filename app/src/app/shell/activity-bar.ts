import { CdkMenu, CdkMenuItem, CdkMenuItemCheckbox, CdkMenuTrigger } from '@angular/cdk/menu';
import { ConnectedPosition } from '@angular/cdk/overlay';
import { NgTemplateOutlet } from '@angular/common';
import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, ElementRef, inject, signal } from '@angular/core';
import { ActivityItems } from '../extensions/activity-items';
import { ariaKeyshortcuts, formatKeybinding } from '../extensions/keybinding';
import { ImtActivityItem, ImtActivityMenuItem, ImtActivityTarget } from '../extensions/tokens';
import { Icon } from '../shared/icon';
import { fittingTopItems } from './activity-overflow';

/** Fly-outs open to the right of their button, top-aligned, or bottom-aligned near the window's bottom edge. */
export const FLYOUT_POSITIONS: ConnectedPosition[] = [
  { originX: 'end', originY: 'top', overlayX: 'start', overlayY: 'top', offsetX: 6 },
  { originX: 'end', originY: 'bottom', overlayX: 'start', overlayY: 'bottom', offsetX: 6 },
];

/**
 * The left activity bar: one button per `ImtActivityItem` (built-in features and plugins contribute them through
 * `IMT_ACTIVITY_ITEMS`), in order, with Settings and other bottom items pinned below. Items with a menu open a CDK
 * fly-out to the right. When the window is too short, the last top items collapse into a More fly-out.
 */
@Component({
  selector: 'imt-activity-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, NgTemplateOutlet, CdkMenu, CdkMenuItem, CdkMenuItemCheckbox, CdkMenuTrigger],
  template: `
    @for (item of shown(); track item.id) {
      <ng-container *ngTemplateOutlet="button; context: { $implicit: item }" />
    }
    @if (overflow().length) {
      <button
        type="button"
        class="act"
        data-activity-id="more"
        aria-label="More"
        i18n-aria-label="@@shell.activityBar.more"
        title="More"
        i18n-title="@@shell.activityBar.moreTitle"
        [cdkMenuTriggerFor]="more"
        [cdkMenuPosition]="positions"
        #moreTrigger="cdkMenuTriggerFor"
        [class.on]="moreTrigger.isOpen()"
        (keydown.arrowright)="openWithKeyboard(moreTrigger, $event)"
      >
        <imt-icon name="more" [size]="18" [stroke]="2.6" />
      </button>
    }
    <div class="spacer"></div>
    @for (item of items.bottom(); track item.id) {
      <ng-container *ngTemplateOutlet="button; context: { $implicit: item }" />
    }

    <ng-template #button let-item>
      @if (items.hasMenu(item)) {
        <button
          type="button"
          class="act"
          [attr.data-activity-id]="item.id"
          [attr.aria-label]="ariaLabel(item)"
          [title]="tooltip(item)"
          [attr.aria-keyshortcuts]="keys(item)"
          [cdkMenuTriggerFor]="flyout"
          [cdkMenuTriggerData]="{ $implicit: item }"
          [cdkMenuPosition]="positions"
          #trigger="cdkMenuTriggerFor"
          [class.on]="trigger.isOpen() || anyChecked(item)"
          (keydown.arrowright)="openWithKeyboard(trigger, $event)"
        >
          <ng-container *ngTemplateOutlet="face; context: { $implicit: item }" />
        </button>
      } @else {
        <button
          type="button"
          class="act"
          [attr.data-activity-id]="item.id"
          [attr.aria-label]="ariaLabel(item)"
          [title]="tooltip(item)"
          [attr.aria-keyshortcuts]="keys(item)"
          [attr.aria-pressed]="item.toggle ? items.isActive(item) : null"
          [attr.aria-disabled]="items.disabledReason(item) ? 'true' : null"
          [class.on]="items.isActive(item)"
          (click)="activate(item)"
        >
          <ng-container *ngTemplateOutlet="face; context: { $implicit: item }" />
        </button>
      }
    </ng-template>

    <ng-template #face let-item>
      <imt-icon [name]="item.icon" [size]="18" />
      @if (items.pulse(item)) {
        <span class="pulse" aria-hidden="true"></span>
      }
      @if (items.badge(item); as badge) {
        <span class="badge" aria-hidden="true">{{ badge }}</span>
      }
    </ng-template>

    <!-- Entries are read on every render: CDK keeps the data a menu was first opened with. -->
    <ng-template #flyout let-item>
      <div cdkMenu class="menu" [attr.aria-label]="item.label" [attr.data-activity-menu]="item.id">
        @for (entry of items.menu(item); track entry.id) {
          @if (entry.checked === undefined) {
            <button type="button" cdkMenuItem class="mi" [cdkMenuitemTypeaheadLabel]="entry.label" [attr.aria-keyshortcuts]="keys(entry)" (cdkMenuItemTriggered)="items.run(entry.run)">
              <ng-container *ngTemplateOutlet="entryFace; context: { $implicit: entry, checked: false }" />
            </button>
          } @else {
            <button
              type="button"
              cdkMenuItemCheckbox
              class="mi"
              [cdkMenuItemChecked]="entry.checked"
              [cdkMenuitemTypeaheadLabel]="entry.label"
              [attr.aria-keyshortcuts]="keys(entry)"
              (cdkMenuItemTriggered)="items.run(entry.run)"
            >
              <ng-container *ngTemplateOutlet="entryFace; context: { $implicit: entry, checked: entry.checked }" />
            </button>
          }
        }
      </div>
    </ng-template>

    <ng-template #more>
      <div cdkMenu class="menu" aria-label="More" i18n-aria-label="@@shell.activityBar.moreMenu" data-activity-menu="more">
        @for (item of overflow(); track item.id) {
          @if (items.hasMenu(item)) {
            <button type="button" cdkMenuItem class="mi" [cdkMenuitemTypeaheadLabel]="item.label" [cdkMenuTriggerFor]="flyout" [cdkMenuTriggerData]="{ $implicit: item }">
              <ng-container *ngTemplateOutlet="entryFace; context: { $implicit: item, checked: false, submenu: true }" />
            </button>
          } @else if (item.toggle) {
            <button
              type="button"
              cdkMenuItemCheckbox
              class="mi"
              [cdkMenuItemChecked]="items.isActive(item)"
              [cdkMenuItemDisabled]="!!items.disabledReason(item)"
              [cdkMenuitemTypeaheadLabel]="item.label"
              [title]="tooltip(item)"
              [attr.aria-keyshortcuts]="keys(item)"
              (cdkMenuItemTriggered)="activate(item)"
            >
              <ng-container *ngTemplateOutlet="entryFace; context: { $implicit: item, checked: items.isActive(item) }" />
            </button>
          } @else {
            <button
              type="button"
              cdkMenuItem
              class="mi"
              [cdkMenuItemDisabled]="!!items.disabledReason(item)"
              [cdkMenuitemTypeaheadLabel]="item.label"
              [title]="tooltip(item)"
              [attr.aria-keyshortcuts]="keys(item)"
              (cdkMenuItemTriggered)="activate(item)"
            >
              <ng-container *ngTemplateOutlet="entryFace; context: { $implicit: item, checked: items.isActive(item) }" />
            </button>
          }
        }
      </div>
    </ng-template>

    <ng-template #entryFace let-entry let-checked="checked" let-submenu="submenu">
      <span class="check" aria-hidden="true">
        @if (checked) {
          <imt-icon name="check" [size]="14" />
        }
      </span>
      <imt-icon class="mi-icon" [name]="entry.icon ?? ''" [size]="14" />
      <span class="label">{{ entry.label }}</span>
      @if (submenu) {
        <span class="hint" aria-hidden="true">›</span>
      } @else if (hint(entry); as h) {
        <span class="hint" aria-hidden="true">{{ h }}</span>
      }
    </ng-template>
  `,
  styles: `
    :host { width: 48px; flex: none; display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 8px 0; background: var(--activity-bg); border-inline-end: 1px solid var(--border); overflow: hidden; }
    .act { position: relative; width: 36px; height: 36px; flex: none; border-radius: 8px; border: 0; background: transparent; color: var(--faint); display: flex; align-items: center; justify-content: center; cursor: pointer; }
    .act:hover { color: var(--text); }
    .act:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; color: var(--text); }
    .act.on { background: var(--bg-active); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
    .act[aria-disabled='true'] { opacity: 0.45; cursor: default; }
    .act[aria-disabled='true']:hover { color: var(--faint); }
    .spacer { flex: 1; min-height: 0; }
    .pulse { position: absolute; top: 6px; inset-inline-end: 6px; width: 7px; height: 7px; border-radius: 50%; background: var(--accent); animation: imt-activity-pulse 1.6s ease-in-out infinite; }
    @keyframes imt-activity-pulse { 0%, 100% { opacity: 1; transform: scale(1); } 50% { opacity: 0.35; transform: scale(0.7); } }
    @media (prefers-reduced-motion: reduce) { .pulse { animation: none; } }
    .badge {
      position: absolute; bottom: 2px; inset-inline-end: 1px; min-width: 14px; height: 14px; padding: 0 3px; box-sizing: border-box; border-radius: 7px;
      background: var(--accent); color: var(--accent-ink); font-size: var(--fs-2xs); font-weight: 600; line-height: 14px; text-align: center;
    }
    .menu {
      min-width: 220px; max-width: min(360px, calc(100vw - 16px)); max-height: calc(100vh - 32px); overflow-y: auto; overscroll-behavior: contain;
      display: flex; flex-direction: column; padding: 4px; border-radius: 8px;
      background: var(--tooltip-bg); border: 1px solid var(--border-3); box-shadow: var(--shadow); color: var(--text-2); font-size: var(--fs-md);
    }
    .menu:focus { outline: none; }
    .mi {
      display: flex; align-items: center; gap: 8px; flex: none; min-height: 28px; padding: 0 10px 0 6px; border: 0; border-radius: 5px;
      background: transparent; color: var(--text-2); cursor: pointer; text-align: start; font: inherit; white-space: nowrap;
    }
    .mi:hover, .mi:focus, .mi[aria-expanded='true'] { background: var(--bg-active); color: var(--text); outline: none; }
    .mi:focus-visible { box-shadow: inset 0 0 0 1px var(--accent); }
    .mi[aria-disabled='true'] { opacity: 0.5; cursor: default; }
    .check { width: 14px; flex: none; display: inline-flex; color: var(--accent); }
    .mi-icon { color: var(--muted); }
    .label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
    .hint { flex: none; margin-inline-start: 24px; color: var(--muted); font-family: var(--font-mono); font-size: var(--fs-xs); }
  `,
})
export class ActivityBar {
  protected readonly items = inject(ActivityItems);
  protected readonly positions = FLYOUT_POSITIONS;
  /** The bar's height in px (0 until measured). */
  protected readonly barHeight = signal(0);

  private readonly fitting = computed(() => fittingTopItems(this.items.top().length, this.items.bottom().length, this.barHeight()));
  protected readonly shown = computed(() => this.items.top().slice(0, this.fitting()));
  protected readonly overflow = computed(() => this.items.top().slice(this.fitting()));

  constructor() {
    const host = inject<ElementRef<HTMLElement>>(ElementRef).nativeElement;
    const destroyRef = inject(DestroyRef);
    afterNextRender(() => {
      if (typeof ResizeObserver !== 'function') return;
      const observer = new ResizeObserver(() => this.barHeight.set(host.clientHeight));
      observer.observe(host);
      destroyRef.onDestroy(() => observer.disconnect());
    });
  }

  /** Shows the view or runs the command, unless the item is inert. */
  protected activate(item: ImtActivityItem): void {
    if (this.items.disabledReason(item) || this.items.hasMenu(item)) return;
    this.items.run(item.run as ImtActivityTarget);
  }

  /** ArrowRight on a fly-out button opens it with the first entry focused (Enter, Space and ArrowDown do too). */
  protected openWithKeyboard(trigger: CdkMenuTrigger, event: Event): void {
    event.preventDefault();
    trigger.open();
    trigger.getMenu()?.focusFirstItem('keyboard');
  }

  /** A fly-out button is highlighted while one of its views is open. */
  protected anyChecked(item: ImtActivityItem): boolean {
    return this.items.menu(item).some((e) => e.checked);
  }

  /**
   * The button's name with its badge ("Notifications, 2"): the count it shows is part of its name (WCAG 2.5.3). Not in
   * parentheses: name-in-label checks (axe) ignore parentheticals, so "Notifications (2)" would not contain the visible "2".
   */
  protected ariaLabel(item: ImtActivityItem): string {
    const badge = this.items.badge(item);
    return badge === null ? item.label : $localize`:Activity bar button name with its badge@@shell.activityBar.labelBadge:${item.label}:label:, ${badge}:badge:`;
  }

  /** "Label (Ctrl+B)", with the reason when the item is inert. */
  protected tooltip(item: ImtActivityItem): string {
    const keys = this.hint(item);
    const label = keys ? $localize`:Activity bar tooltip with its shortcut@@shell.activityBar.tooltipKeys:${item.label}:label: (${keys}:keys:)` : item.label;
    const reason = this.items.disabledReason(item);
    return reason ? $localize`:Activity bar tooltip of an unavailable button@@shell.activityBar.tooltipReason:${label}:label: — ${reason}:reason:` : label;
  }

  protected hint(item: ImtActivityItem | ImtActivityMenuItem): string {
    const binding = this.items.keybinding(item);
    return binding ? formatKeybinding(binding) : '';
  }

  protected keys(item: ImtActivityItem | ImtActivityMenuItem): string | null {
    const binding = this.items.keybinding(item);
    return binding ? ariaKeyshortcuts(binding) : null;
  }
}
