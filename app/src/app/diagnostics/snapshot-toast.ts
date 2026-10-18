import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { CommandRegistry } from '../extensions/command-registry';
import { ErrorSnapshotService } from './error-snapshot.service';
import { SnapshotBackend } from './snapshot-backend';
import { SETTINGS_COMMAND } from './error-snapshots-view';
import { REPORT_PROBLEM_COMMAND, SNAPSHOT_NOTICE } from './snapshot.types';

/** "Error captured — View", plus the one-time notice that the screenshot is local. Mounted once over the app. */
@Component({
  selector: 'imt-snapshot-toast',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { 'data-snapshot-ignore': '' },
  template: `
    @if (service.toast(); as t) {
      <div class="toast" role="status" aria-live="polite" data-testid="snapshot-toast">
        @if (t.kind === 'captured') {
          <span class="line"><strong i18n="@@diagnostics.toast.captured">Error captured</strong> — <button type="button" class="link" (click)="view()" i18n="Open the captured snapshot|Button@@diagnostics.toast.view">View</button> ·
            <button type="button" class="link" (click)="report()" data-testid="toast-report-problem" i18n="@@diagnostics.toast.report">Report a problem…</button></span>
          @if (t.firstTime) {
            <span class="note" data-testid="snapshot-notice">{{ notice }}
              <button type="button" class="link" (click)="folder()" i18n="@@diagnostics.toast.openFolder">Open folder</button> ·
              <button type="button" class="link" (click)="settings()" i18n="@@diagnostics.toast.settings">Settings…</button></span>
          }
        } @else {
          <span class="line" i18n="@@diagnostics.toast.failed">The screenshot could not be captured.</span>
        }
        <button type="button" class="x" aria-label="Dismiss" i18n-aria-label="@@diagnostics.toast.dismiss" (click)="service.dismissToast()">×</button>
      </div>
    }
  `,
  styles: `
    :host { position: fixed; right: 16px; bottom: 32px; z-index: 2000; }
    .toast { position: relative; max-width: 380px; padding: 10px 32px 10px 12px; border: 1px solid var(--border-2); border-left: 3px solid var(--lvl-warn); border-radius: 8px; background: var(--bg-3); color: var(--text); box-shadow: 0 6px 24px rgba(0, 0, 0, 0.35); display: flex; flex-direction: column; gap: 4px; font-size: 12.5px; }
    .note { color: var(--text-2); line-height: 1.4; }
    .link { all: unset; cursor: pointer; color: var(--accent); text-decoration: underline; }
    .link:focus-visible, .x:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
    .x { all: unset; position: absolute; top: 6px; right: 8px; cursor: pointer; color: var(--text-2); font-size: 16px; line-height: 1; }
  `,
})
export class SnapshotToast {
  protected readonly service = inject(ErrorSnapshotService);
  private readonly commands = inject(CommandRegistry);
  private readonly backend = inject(SnapshotBackend);
  protected readonly notice = SNAPSHOT_NOTICE;

  protected view(): void {
    void this.commands.run('diagnostics.snapshots.show');
    this.service.dismissToast();
  }

  protected report(): void {
    void this.commands.run(REPORT_PROBLEM_COMMAND);
    this.service.dismissToast();
  }

  protected folder(): void {
    void this.backend.openFolder().catch(() => undefined);
  }

  protected settings(): void {
    void this.commands.run(SETTINGS_COMMAND);
  }
}
