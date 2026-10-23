import { computed, inject, Injectable } from '@angular/core';
import { CommandRegistry } from '../extensions/command-registry';
import { HostAction, HostNotification, NotificationSource } from '../health/notification-host.model';
import { ErrorSnapshotService } from './error-snapshot.service';
import { SETTINGS_COMMAND } from './error-snapshots-view';
import { SnapshotBackend } from './snapshot-backend';
import { REPORT_PROBLEM_COMMAND, SNAPSHOT_NOTICE } from './snapshot.types';

/**
 * After an error: "Something went wrong in Timeline." with Report a problem… and Details (what Debrief did about it: a
 * local screenshot, nothing sent), as a host notification.
 */
@Injectable({ providedIn: 'root' })
export class SnapshotSource implements NotificationSource {
  private readonly service = inject(ErrorSnapshotService);
  private readonly commands = inject(CommandRegistry);
  private readonly backend = inject(SnapshotBackend);

  readonly items = computed<HostNotification[]>(() => {
    const t = this.service.toast();
    if (!t) return [];
    const dismiss = () => this.service.dismissToast();
    const run = (id: string): HostAction['run'] => () => {
      void this.commands.run(id);
      dismiss();
    };
    if (t.kind !== 'captured') {
      return [{ cause: 'diagnostics.snapshot', severity: 'warning', title: $localize`:@@diagnostics.toast.failed:The screenshot could not be captured.`, testId: 'snapshot-toast', dismiss }];
    }
    if (t.manual) {
      return [
        {
          cause: 'diagnostics.snapshot',
          severity: 'success',
          title: $localize`:@@diagnostics.toast.manual:Diagnostic screenshot captured.`,
          testId: 'snapshot-toast',
          actions: [{ label: $localize`:Open the captured snapshot|Button@@diagnostics.toast.view:View`, run: run('diagnostics.snapshots.show') }],
          dismiss,
        },
      ];
    }
    return [
      {
        cause: 'diagnostics.snapshot',
        severity: 'warning',
        title: t.where ? $localize`:@@diagnostics.toast.wrongIn:Something went wrong in ${t.where}:panel:.` : $localize`:@@diagnostics.toast.wrong:Something went wrong.`,
        instance: t.id ?? '',
        details: SNAPSHOT_NOTICE,
        testId: 'snapshot-toast',
        actions: [
          { label: $localize`:@@diagnostics.toast.report:Report a problem…`, testId: 'toast-report-problem', run: run(REPORT_PROBLEM_COMMAND) },
          { label: $localize`:@@diagnostics.toast.viewSnapshot:View snapshot`, kind: 'ghost', run: run('diagnostics.snapshots.show') },
          { label: $localize`:@@diagnostics.toast.openFolder:Open folder`, kind: 'ghost', run: () => void this.backend.openFolder().catch(() => undefined) },
          { label: $localize`:@@diagnostics.toast.settings:Settings…`, kind: 'ghost', run: () => void this.commands.run(SETTINGS_COMMAND) },
        ],
        dismiss,
      },
    ];
  });
}
