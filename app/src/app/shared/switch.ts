import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';

let nextId = 0;

/**
 * An on/off switch with its label: a native checkbox exposed as `role="switch"` (keyboard, forms and label clicks
 * work as usual), drawn as a track and knob. On and off differ by knob position and track fill, not by colour alone.
 * The projected content is the label; it wraps beside the switch, never under it.
 */
@Component({
  selector: 'imt-switch',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { class: 'imt-switch', '[class.small]': 'size() === "small"' },
  template: `
    <label class="sw" [for]="inputId()">
      <input
        type="checkbox"
        role="switch"
        [id]="inputId()"
        [checked]="checked()"
        [disabled]="disabled()"
        [attr.aria-describedby]="describedBy()"
        [attr.aria-invalid]="invalid() || null"
        (change)="checkedChange.emit($any($event.target).checked)"
      />
      <span class="text"><ng-content /></span>
    </label>
  `,
  styles: `
    :host { display: inline-flex; min-width: 0; max-width: 100%; --sw-w: 30px; --sw-h: 18px; --sw-knob: 12px; }
    :host(.small) { --sw-w: 26px; --sw-h: 16px; --sw-knob: 10px; font-size: 12px; }
    .sw { display: inline-flex; align-items: center; gap: 8px; min-width: 0; cursor: pointer; }
    .text { min-width: 0; overflow-wrap: anywhere; }
    input {
      appearance: none; -webkit-appearance: none; flex: none; position: relative; margin: 0; cursor: inherit;
      width: var(--sw-w); height: var(--sw-h); border-radius: calc(var(--sw-h) / 2);
      background: var(--bg-3); border: 1px solid var(--border-3); box-sizing: border-box;
      transition: background-color 0.12s, border-color 0.12s;
    }
    input::before {
      content: ''; position: absolute; top: 50%; left: 2px; width: var(--sw-knob); height: var(--sw-knob);
      border-radius: 50%; background: var(--muted); transform: translateY(-50%); transition: left 0.12s, background-color 0.12s;
    }
    input:checked { background: var(--accent); border-color: var(--accent); }
    input:checked::before { left: calc(var(--sw-w) - var(--sw-knob) - 4px); background: var(--accent-ink, #fff); }
    input:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
    input:disabled { opacity: 0.5; }
    .sw:has(input:disabled) { cursor: default; }
    @media (prefers-reduced-motion: reduce) { input, input::before { transition: none; } }
  `,
})
export class Switch {
  readonly checked = input(false);
  readonly disabled = input(false);
  readonly invalid = input(false);
  readonly describedBy = input<string | null>(null);
  readonly size = input<'normal' | 'small'>('normal');
  /** Id of the inner checkbox (for `aria-describedby` targets and external labels). */
  readonly inputId = input(`imt-switch-${nextId++}`);
  readonly checkedChange = output<boolean>();
}
