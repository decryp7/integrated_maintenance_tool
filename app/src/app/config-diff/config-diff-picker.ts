import { Injectable } from '@angular/core';
import { isTauri } from '../core/platform';
import { DiffExportFormat } from '../core/config-diff.types';
import { nativePickOne } from '../core/native-dialog';

export type Side = 'left' | 'right';

/* v8 ignore start: the native dialogs run only inside Tauri; the browser branches are tested */
/** Native dialogs for the diff view. In a plain browser it returns demo paths the mock understands. */
@Injectable({ providedIn: 'root' })
export class ConfigDiffPicker {
  /** A configuration file for one side; null when cancelled. */
  async pickFile(side: Side): Promise<string | null> {
    if (!isTauri()) return `/demo/config/${side}/appsettings.json`;
    return nativePickOne({ kind: 'openFile', title: side === 'left' ? $localize`:@@configDiff.picker.leftFile:Original (left) file` : $localize`:@@configDiff.picker.rightFile:Changed (right) file` });
  }

  /** A snapshot folder for one side; null when cancelled. */
  async pickFolder(side: Side): Promise<string | null> {
    if (!isTauri()) return `/demo/snapshots/${side === 'left' ? 'monday' : 'tuesday'}`;
    return nativePickOne({ kind: 'openFolder', title: side === 'left' ? $localize`:@@configDiff.picker.leftFolder:Earlier snapshot folder` : $localize`:@@configDiff.picker.rightFolder:Later snapshot folder` });
  }

  /** Where to export the diff; null when cancelled. */
  async pickExport(format: DiffExportFormat): Promise<string | null> {
    const name = `config-diff.${format}`;
    if (!isTauri()) return `/demo/exports/${name}`;
    return nativePickOne({ kind: 'save', title: $localize`:@@configDiff.picker.exportDiff:Export diff`, defaultName: name, filters: [{ name: format.toUpperCase(), extensions: [format] }] });
  }
}
/* v8 ignore stop */
