import { ChangeDetectionStrategy, Component, computed, inject, linkedSignal } from '@angular/core';
import { CommandRegistry } from '../extensions/command-registry';
import { ErrorSnapshotService } from './error-snapshot.service';
import { SnapshotBackend } from './snapshot-backend';
import { SETTINGS_COMMAND } from './error-snapshots-view';
import { REPORT_PROBLEM_COMMAND, SNAPSHOT_NOTICE } from './snapshot.types';

/**
 * After an error: "Something went wrong in Timeline." with Details and Report a problem…; Details holds what Debrief did
 * about it (a local screenshot, nothing sent) and its links. Mounted once in the toast stack.
 */
@Component({
  selector: 'debrief-snapshot-toast',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { 'data-snapshot-ignore': '' },
  template: `
    @if (service.toast(); as t) {
      <div class="toast" role="status" aria-live="polite" data-toast data-testid="snapshot-toast">
        @if (t.kind === 'captured') {
          @if (t.manual) {
            <span class="line"><strong i18n="@@diagnostics.toast.manual">Diagnostic screenshot captured.</strong>
              <button type="button" class="link" (click)="view()" i18n="Open the captured snapshot|Button@@diagnostics.toast.view">View</button></span>
          } @else {
            <strong class="line">{{ headline() }}</strong>
            <span class="actions">
              <button type="button" class="link" [attr.aria-expanded]="details()" aria-controls="debrief-snapshot-details" (click)="details.set(!details())" data-testid="snapshot-details-toggle" i18n="@@diagnostics.toast.details">Details</button>
              <button type="button" class="link" (click)="report()" data-testid="toast-report-problem" i18n="@@diagnostics.toast.report">Report a problem…</button>
            </span>
            @if (details()) {
              <span class="note" id="debrief-snapshot-details" data-testid="snapshot-notice">{{ notice }}
                <button type="button" class="link" (click)="view()" i18n="@@diagnostics.toast.viewSnapshot">View snapshot</button> ·
                <button type="button" class="link" (click)="folder()" i18n="@@diagnostics.toast.openFolder">Open folder</button> ·
                <button type="button" class="link" (click)="settings()" i18n="@@diagnostics.toast.settings">Settings…</button></span>
            }
          }
        } @else {
          <span class="line" i18n="@@diagnostics.toast.failed">The screenshot could not be captured.</span>
        }
        <button type="button" class="x" aria-label="Dismiss" i18n-aria-label="@@diagnostics.toast.dismiss" (click)="service.dismissToast()">×</button>
      </div>
    }
  `,
  styles: `
    :host { display: block; }
    .actions { display: flex; gap: 12px; }
    .toast { position: relative; width: 380px; max-width: calc(100vw - 32px); box-sizing: border-box; padding: 10px 32px 10px 12px; border: 1px solid var(--border-2); border-left: 3px solid var(--lvl-warn); border-radius: 8px; background: var(--bg-3); color: var(--text); box-shadow: var(--shadow-pop); display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-md); }
    .note { color: var(--text-2); line-height: 1.4; }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; }
    .link:focus-visible, .x:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
    .x { all: unset; position: absolute; top: 6px; right: 8px; cursor: pointer; color: var(--text-2); font-size: var(--fs-lg); line-height: 1; }
  `,
})
export class SnapshotToast {
  protected readonly service = inject(ErrorSnapshotService);
  private readonly commands = inject(CommandRegistry);
  private readonly backend = inject(SnapshotBackend);
  protected readonly notice = SNAPSHOT_NOTICE;
  /** Details are folded for each new toast. */
  protected readonly details = linkedSignal({ source: () => this.service.toast(), computation: () => false });
  protected readonly headline = computed(() => {
    const where = this.service.toast()?.where;
    return where ? $localize`:@@diagnostics.toast.wrongIn:Something went wrong in ${where}:panel:.` : $localize`:@@diagnostics.toast.wrong:Something went wrong.`;
  });

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
