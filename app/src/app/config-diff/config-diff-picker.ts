import { Injectable } from '@angular/core';
import { isTauri } from '../core/platform';
import { DiffExportFormat } from '../core/config-diff.types';

export type Side = 'left' | 'right';

/* v8 ignore start: the native dialogs run only inside Tauri; the browser branches are tested */
/** Native dialogs for the diff view. In a plain browser it returns demo paths the mock understands. */
@Injectable({ providedIn: 'root' })
export class ConfigDiffPicker {
  /** A configuration file for one side; null when cancelled. */
  async pickFile(side: Side): Promise<string | null> {
    if (!isTauri()) return `/demo/config/${side}/appsettings.json`;
    const { open } = await import('@tauri-apps/plugin-dialog');
    const result = await open({ multiple: false, directory: false, title: side === 'left' ? 'Original (left) file' : 'Changed (right) file' });
    return typeof result === 'string' ? result : null;
  }

  /** A snapshot folder for one side; null when cancelled. */
  async pickFolder(side: Side): Promise<string | null> {
    if (!isTauri()) return `/demo/snapshots/${side === 'left' ? 'monday' : 'tuesday'}`;
    const { open } = await import('@tauri-apps/plugin-dialog');
    const result = await open({ multiple: false, directory: true, title: side === 'left' ? 'Earlier snapshot folder' : 'Later snapshot folder' });
    return typeof result === 'string' ? result : null;
  }

  /** Where to export the diff; null when cancelled. */
  async pickExport(format: DiffExportFormat): Promise<string | null> {
    const name = `config-diff.${format}`;
    if (!isTauri()) return `/demo/exports/${name}`;
    const { save } = await import('@tauri-apps/plugin-dialog');
    return save({ title: 'Export diff', defaultPath: name, filters: [{ name: format.toUpperCase(), extensions: [format] }] });
  }
}
/* v8 ignore stop */
