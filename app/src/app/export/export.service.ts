import { inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { DialogService } from '../ui/dialog.service';
import { ExportDialog } from './export-dialog';

export { exporterForPath } from './exporters';

/** Opens the export dialog (format, scope, order, event count, then the save dialog). */
@Injectable({ providedIn: 'root' })
export class ExportService {
  private readonly store = inject(WorkspaceStore);
  private readonly dialogs = inject(DialogService);

  async exportCurrent(): Promise<void> {
    if (!this.store.summary()?.exporters.length) throw new Error($localize`:@@export.service.noFormats:No export formats are available.`);
    await this.dialogs.open(ExportDialog, { title: $localize`:@@export.service.title:Export events`, width: 480 }).closed;
  }
}
