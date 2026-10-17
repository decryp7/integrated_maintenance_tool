import { Injectable } from '@angular/core';
import { isTauri } from '../core/platform';

/** Where an export goes: the native save dialog in Tauri; a default name in a browser. Replaceable in tests. */
@Injectable({ providedIn: 'root' })
export class ExportTarget {
  /** The chosen `.zip` path, or null when cancelled. */
  async pickZip(defaultName: string): Promise<string | null> {
    if (!isTauri()) return defaultName;
    const { save } = await import('@tauri-apps/plugin-dialog');
    return save({ title: 'Export diagnostics', defaultPath: defaultName, filters: [{ name: 'Zip archive', extensions: ['zip'] }] });
  }
}
