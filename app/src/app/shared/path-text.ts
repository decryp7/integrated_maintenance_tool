import { ChangeDetectionStrategy, Component, computed, input, signal } from '@angular/core';
import { copyText } from '../tools/clipboard';
import { splitForEllipsis } from './format/middle-ellipsis';
import { Icon } from './icon';

/**
 * A path, file name or other long identifier on one line that shortens in the middle when its container is narrow:
 * the start stays, an ellipsis replaces the rest of the start, and the file name (or the last characters) always
 * stays visible. The full value is the tooltip and, with `copy`, one click on a Copy button (shown on hover and focus).
 * Fills the width it is given: put it in a flex or grid child with `min-width: 0`.
 */
@Component({
  selector: 'debrief-path-text',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `<span class="text" [attr.title]="whole()" data-testid="path-text"><span class="head">{{ parts().head }}</span><span class="tail">{{ parts().tail }}</span></span>
    @if (copy()) {
      <button class="copy" type="button" [attr.aria-label]="copied() ? copiedLabel : copyLabel" [attr.title]="copied() ? copiedLabel : copyLabel" (click)="doCopy($event)">
        <debrief-icon [name]="copied() ? 'check' : 'copy'" [size]="12" />
      </button>
    }`,
  styles: `
    :host { display: flex; align-items: center; gap: 4px; min-width: 0; max-width: 100%; }
    .text { display: flex; min-width: 0; flex: 0 1 auto; white-space: nowrap; }
    .head { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
    .tail { flex: 0 0.001 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; }
    .copy { flex: none; display: inline-flex; padding: 2px; border: 0; border-radius: var(--r-1, 3px); background: transparent; color: var(--muted); cursor: pointer; opacity: 0; }
    :host(:hover) .copy, .copy:focus-visible { opacity: 1; }
    .copy:hover { color: var(--text); background: var(--bg-hover); }
    .copy:focus-visible { outline: 2px solid var(--focus-ring); }
    @media (hover: none) { .copy { opacity: 0.5; } }
  `,
})
export class PathText {
  /** The text shown (shortened in the middle when it does not fit). */
  readonly value = input.required<string>();
  /** What the tooltip shows and Copy copies when it is more than `value` (a file name shown for its full path). */
  readonly full = input<string | null>(null);
  /** Characters of the end that are never cut (a longer file name keeps its last ones). */
  readonly keepTail = input(20);
  /** Shows a Copy button for the full value. */
  readonly copy = input(false);

  protected readonly parts = computed(() => splitForEllipsis(this.value(), this.keepTail()));
  protected readonly whole = computed(() => this.full() ?? this.value());
  protected readonly copied = signal(false);
  protected readonly copyLabel = $localize`:@@shared.pathText.copy:Copy the full path`;
  protected readonly copiedLabel = $localize`:@@shared.pathText.copied:Copied`;

  protected async doCopy(event: Event): Promise<void> {
    event.stopPropagation();
    if (!(await copyText(this.whole()))) return;
    this.copied.set(true);
    setTimeout(() => this.copied.set(false), 1500);
  }
}
