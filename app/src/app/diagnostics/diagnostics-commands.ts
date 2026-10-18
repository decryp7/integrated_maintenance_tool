import { inject, Injectable, Injector } from '@angular/core';
import { ImtCommand, ImtView } from '../extensions/tokens';
import { describeError } from '../core/logger';
import { DialogService } from '../ui/dialog.service';
import { ErrorSnapshotService } from './error-snapshot.service';
import { SnapshotBackend } from './snapshot-backend';
import { ACTIVITY_LOG_COMMAND, REPORT_PROBLEM_COMMAND } from './snapshot.types';

export { ACTIVITY_LOG_COMMAND, REPORT_PROBLEM_COMMAND };

/** The view that lists the saved error snapshots. */
export const DIAGNOSTICS_VIEWS: ImtView[] = [
  {
    id: 'error-snapshots',
    title: $localize`:@@diagnostics.commands.view.title:Error snapshots`,
    category: 'Diagnostics',
    icon: 'file',
    defaultPosition: 'center',
    component: () => import('./error-snapshots-view').then((m) => m.ErrorSnapshotsView),
  },
];

/** Keyboard shortcut of the manual capture. */
export const CAPTURE_KEYBINDING = 'Mod+Shift+F12';

@Injectable({ providedIn: 'root' })
export class DiagnosticsCommands {
  private readonly snapshots = inject(ErrorSnapshotService);
  private readonly dialogs = inject(DialogService);
  private readonly injector = inject(Injector);

  readonly commands: ImtCommand[] = [
    {
      id: 'diagnostics.capture',
      title: $localize`:@@diagnostics.commands.capture:Capture diagnostic screenshot`,
      category: $localize`:@@diagnostics.commands.category.tools:Tools`,
      keybinding: CAPTURE_KEYBINDING,
      run: async () => void (await this.snapshots.manual()),
    },
    { id: 'diagnostics.snapshots.show', title: $localize`:@@diagnostics.commands.showSnapshots:Show error snapshots`, category: $localize`:@@diagnostics.commands.category.tools:Tools`, run: (ctx) => ctx.showView('error-snapshots') },
    {
      id: REPORT_PROBLEM_COMMAND,
      title: $localize`:@@diagnostics.commands.report:Report a problem…`,
      category: $localize`:@@diagnostics.commands.category.help:Help`,
      run: async () => {
        // Loaded on demand: the dialog is not part of the first paint.
        const { ReportProblemDialog } = await import('./report-problem-dialog');
        this.dialogs.open(ReportProblemDialog, { title: $localize`:@@diagnostics.commands.report.dialogTitle:Report a problem`, width: 680 });
      },
    },
    {
      id: ACTIVITY_LOG_COMMAND,
      title: $localize`:@@diagnostics.commands.activityLog:Show activity log`,
      category: $localize`:@@diagnostics.commands.category.help:Help`,
      run: async (ctx) => {
        try {
          await this.injector.get(SnapshotBackend).openActivityLog();
        } catch (e) {
          ctx.notifyError($localize`:@@diagnostics.commands.activityLogFailed:Could not open the activity log: ${describeError(e)}:error:`);
        }
      },
    },
  ];
}
