import { ChangeDetectionStrategy, Component, computed, input, signal } from '@angular/core';

let nextId = 0;

/** A small "?" button that toggles a one- or two-sentence explanation next to a setting or section. */
@Component({
  selector: 'imt-help',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <button
      type="button"
      class="q"
      [attr.aria-label]="helpLabel()"
      [attr.aria-expanded]="open()"
      [attr.aria-controls]="id"
      title="What is this?" i18n-title="@@ui.help.whatIsThis"
      (click)="open.set(!open())"
      (keydown.escape)="open.set(false)"
    >?</button>
    @if (open()) {
      <span class="tip" role="note" [id]="id">{{ text() }}</span>
    }
  `,
  styles: `
    :host { position: relative; display: inline-flex; }
    .q { width: 18px; height: 18px; padding: 0; border-radius: 50%; border: 1px solid var(--border-3); background: transparent; color: var(--muted); font-size: 11px; line-height: 1; cursor: pointer; }
    .q:hover, .q[aria-expanded='true'] { background: var(--bg-active); color: var(--text); }
    .tip { position: absolute; z-index: 60; top: 24px; inset-inline-start: 0; width: 280px; padding: 8px 10px; border-radius: 8px; background: var(--tooltip-bg); border: 1px solid var(--border-3); box-shadow: var(--shadow); color: var(--text-2); font-size: 12px; font-weight: 400; line-height: 1.45; text-transform: none; letter-spacing: 0; white-space: normal; }
  `,
})
export class HelpHint {
  /** What the hint is about, used in the button's accessible name ("Help: Pipeline"). */
  readonly label = input.required<string>();
  readonly text = input.required<string>();
  protected readonly helpLabel = computed(() => $localize`:@@ui.help.ariaLabel:Help: ${this.label()}:label:`);
  protected readonly open = signal(false);
  protected readonly id = `imt-help-${nextId++}`;
}
