import { inject, Injectable, Injector, signal } from '@angular/core';
import { describeError } from '../core/logger';
import { OverwriteGuard } from '../ui/overwrite-guard';
import { ExportTarget } from './export-target';
import { SnapshotBackend } from './snapshot-backend';
import { JobFollow } from '../jobs/job-follow';

/** Outcome of the last "Save diagnostics…", shown by the banners. */
export interface DiagnosticsExportStatus {
  kind: 'saved' | 'failed';
  text: string;
}

/** `imt-diagnostics-YYYYMMDD-HHMMSS.zip` in local time. */
export function diagnosticsFileName(now: Date = new Date()): string {
  const p = (n: number, w = 2) => String(n).padStart(w, '0');
  return `imt-diagnostics-${p(now.getFullYear(), 4)}${p(now.getMonth() + 1)}${p(now.getDate())}-${p(now.getHours())}${p(now.getMinutes())}${p(now.getSeconds())}.zip`;
}

/** Saves the diagnostics zip (the problem report without a description) where the user chooses. */
@Injectable({ providedIn: 'root' })
export class DiagnosticsExport {
  private readonly injector = inject(Injector);
  private readonly target = inject(ExportTarget);
  private readonly overwrite = inject(OverwriteGuard);

  readonly status = signal<DiagnosticsExportStatus | null>(null);
  readonly busy = signal(false);

  dismiss(): void {
    this.status.set(null);
  }

  /** Asks where to save, then writes the zip (asking before replacing a file). Never rejects. */
  async save(): Promise<void> {
    if (this.busy()) return;
    this.busy.set(true);
    this.status.set(null);
    try {
      const path = await this.target.pickZip(diagnosticsFileName(), $localize`:@@diagnostics.export.title:Save diagnostics`);
      if (path) {
        // Looked up late: the backend is not needed until the user saves.
        const backend = this.injector.get(SnapshotBackend);
        const job = await this.overwrite.write(path, (replace) => backend.saveReport(path, '', replace));
        if (job !== null) {
          this.injector.get(JobFollow).started($localize`:@@diagnostics.export.started:Saving diagnostics — running in Jobs`);
          this.status.set({ kind: 'saved', text: $localize`:@@diagnostics.export.saving:Saving diagnostics to ${path}:path:; a message tells you when the file is ready.` });
        }
      }
    } catch (e) {
      this.status.set({ kind: 'failed', text: $localize`:@@diagnostics.export.failed:Could not save the diagnostics: ${describeError(e)}:error:` });
    } finally {
      this.busy.set(false);
    }
  }
}
