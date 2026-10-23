import { CdkListbox, CdkOption, ListboxValueChangeEvent } from '@angular/cdk/listbox';
import { CdkConnectedOverlay, CdkOverlayOrigin, ConnectedPosition } from '@angular/cdk/overlay';
import { ChangeDetectionStrategy, Component, computed, input, output, signal, viewChild } from '@angular/core';

/** One choice of an `DebriefSelect`. */
export interface SelectOption<T extends string | number = string> {
  value: T;
  label: string;
}

/**
 * A single-choice picker that replaces the native `select`: a trigger styled like `.select` that opens a CDK listbox in
 * a CDK overlay (keyboard, roles and focus from the CDK), themed only by Debrief's CSS tokens in dark and light.
 */
@Component({
  selector: 'debrief-select',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CdkListbox, CdkOption, CdkConnectedOverlay, CdkOverlayOrigin],
  template: `
    <button
      type="button"
      class="select trigger"
      cdkOverlayOrigin
      #origin="cdkOverlayOrigin"
      aria-haspopup="listbox"
      [attr.aria-label]="ariaLabel() ? ariaLabel() + ': ' + current() : null"
      [attr.aria-expanded]="open()"
      [disabled]="disabled()"
      (click)="open.set(!open())"
    >
      <span class="label">{{ current() }}</span><span class="caret" aria-hidden="true">▾</span>
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
      <div class="list" (keydown.escape)="open.set(false)">
        <ul cdkListbox class="options" [attr.aria-label]="ariaLabel() || null" [cdkListboxValue]="[value()]" (cdkListboxValueChange)="changed($event)">
          @for (o of options(); track o.value) {
            <li class="opt" [cdkOption]="o.value">
              <span class="check" aria-hidden="true">{{ o.value === value() ? '✓' : '' }}</span>
              <span>{{ o.label }}</span>
            </li>
          }
        </ul>
      </div>
    </ng-template>
  `,
  styles: `
    :host { display: inline-block; min-width: 0; }
    .trigger { display: inline-flex; align-items: center; justify-content: space-between; gap: 8px; width: 100%; text-align: left; cursor: pointer; }
    .trigger:disabled { opacity: 0.5; cursor: default; }
    .trigger:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
    .label { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .caret { color: var(--muted); font-size: var(--fs-xs); }
    .list { margin: 2px 0 0; padding: 4px; min-width: 160px; max-height: 280px; overflow: auto; border: 1px solid var(--border-3); border-radius: var(--r-2); background: var(--bg-3); box-shadow: var(--shadow-pop); }
    .options { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; outline: none; }
    .opt { display: flex; align-items: center; gap: 8px; min-height: 26px; padding: 0 6px; border-radius: 4px; color: var(--text-2); cursor: pointer; font-size: var(--fs-sm); }
    .opt:hover, .opt.cdk-option-active { background: var(--bg-hover); }
    .opt[aria-selected='true'] { color: var(--text); }
    .check { inline-size: 14px; color: var(--accent); }
  `,
})
export class DebriefSelect {
  readonly options = input.required<readonly SelectOption<string | number>[]>();
  readonly value = input.required<string | number>();
  readonly ariaLabel = input<string>('');
  readonly disabled = input(false);
  readonly valueChange = output<string | number>();

  protected readonly open = signal(false);
  private readonly listbox = viewChild(CdkListbox);
  protected readonly positions: ConnectedPosition[] = [
    { originX: 'start', originY: 'bottom', overlayX: 'start', overlayY: 'top' },
    { originX: 'start', originY: 'top', overlayX: 'start', overlayY: 'bottom' },
  ];
  protected readonly current = computed(() => this.options().find((o) => o.value === this.value())?.label ?? '');

  protected focusList(): void {
    queueMicrotask(() => this.listbox()?.focus());
  }

  protected changed(e: ListboxValueChangeEvent<string | number>): void {
    const v = e.value[0];
    if (v !== undefined) this.valueChange.emit(v);
    this.open.set(false);
  }

  protected outside(e: MouseEvent, trigger: HTMLElement): void {
    if (!(e.target instanceof Node && trigger.contains(e.target))) this.open.set(false);
  }
}
