import { inject, Injectable, Injector } from '@angular/core';
import { DebriefCommand, DebriefView } from '../extensions/tokens';
import { describeError } from '../core/logger';
import { DialogService } from '../ui/dialog.service';
import { DiagnosticsExport } from './diagnostics-export.service';
import { ErrorSnapshotService } from './error-snapshot.service';
import { SnapshotBackend } from './snapshot-backend';
import { ACTIVITY_LOG_COMMAND, REPORT_PROBLEM_COMMAND } from './snapshot.types';

export { ACTIVITY_LOG_COMMAND, REPORT_PROBLEM_COMMAND };

/** The view that lists the saved error snapshots. */
export const DIAGNOSTICS_VIEWS: DebriefView[] = [
  {
    id: 'error-snapshots',
    title: $localize`:@@diagnostics.commands.view.title:Error snapshots`,
    category: 'Diagnostics',
    icon: 'file',
    defaultPosition: 'center', layout: { area: 'status', min: { width: 320, height: 180 }, pref: { width: 480, height: 360 }, priority: 20 },
    component: () => import('./error-snapshots-view').then((m) => m.ErrorSnapshotsView),
  },
  {
    id: 'diagnostics-report',
    title: $localize`:@@diagnostics.commands.reportView.title:Diagnostics report`,
    category: 'Diagnostics',
    icon: 'file',
    defaultPosition: 'center', layout: { area: 'document', min: { width: 420, height: 240 }, pref: { width: 640, height: 400 }, priority: 55 },
    component: () => import('./diagnostics-report-view').then((m) => m.DiagnosticsReportPanel),
  },
];

/** Keyboard shortcut of the manual capture. */
export const CAPTURE_KEYBINDING = 'Mod+Shift+F12';

@Injectable({ providedIn: 'root' })
export class DiagnosticsCommands {
  private readonly snapshots = inject(ErrorSnapshotService);
  private readonly dialogs = inject(DialogService);
  private readonly injector = inject(Injector);
  private readonly exporter = inject(DiagnosticsExport);

  readonly commands: DebriefCommand[] = [
    {
      id: 'diagnostics.capture',
      title: $localize`:@@diagnostics.commands.capture:Capture diagnostic screenshot`,
      category: $localize`:@@diagnostics.commands.category.tools:Tools`,
      keybinding: CAPTURE_KEYBINDING,
      run: async () => void (await this.snapshots.manual()),
    },
    { id: 'diagnostics.snapshots.show', title: $localize`:@@diagnostics.commands.showSnapshots:Show error snapshots`, category: $localize`:@@diagnostics.commands.category.tools:Tools`, view: 'error-snapshots', run: (ctx) => ctx.showView('error-snapshots') },
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
      id: 'diagnostics.saveDiagnostics',
      title: $localize`:@@diagnostics.commands.saveDiagnostics:Save diagnostics…`,
      category: $localize`:@@diagnostics.commands.category.help:Help`,
      run: () => this.exporter.save(),
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
