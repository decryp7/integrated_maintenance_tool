import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { describeError } from '../core/logger';
import { Platform } from '../core/platform';
import { DIALOG_FORM_STYLES } from '../context/dialog-styles';
import { DIALOG_REF } from '../ui/dialog.service';
import { LiveTail } from '../workspace/live-tail';

/** "Open for live tail": pick files or a folder, optionally skipping their history (start at end), and follow them. */
@Component({
  selector: 'imt-open-live-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p>Open log files or a folder and follow them as they grow. Live tail turns on.</p>
    <label class="check">
      <input type="checkbox" data-testid="start-at-end" [checked]="startAtEnd()" (change)="startAtEnd.set($any($event.target).checked)" />
      Live: start at end (only new lines)
    </label>
    <p class="faint small">Without it, the files' existing lines are imported too.</p>
    @if (error(); as e) { <p class="err" role="alert">{{ e }}</p> }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()">Cancel</button>
      <button type="button" class="btn" [disabled]="busy()" (click)="open(false)">Open files…</button>
      <button type="button" class="btn primary" [disabled]="busy()" (click)="open(true)">Open folder…</button>
    </div>
  `,
  styles: [DIALOG_FORM_STYLES, `.small { font-size: 11px; }`],
})
export class OpenLiveDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly platform = inject(Platform);
  private readonly live = inject(LiveTail);

  protected readonly startAtEnd = signal(true);
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);

  protected async open(directory: boolean): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    try {
      const paths = await this.platform.pickPaths(directory);
      if (!paths.length) return;
      this.ref.close(true);
      await this.live.openForLive(paths, this.startAtEnd());
    } catch (e) {
      this.error.set(describeError(e));
    } finally {
      this.busy.set(false);
    }
  }
}
