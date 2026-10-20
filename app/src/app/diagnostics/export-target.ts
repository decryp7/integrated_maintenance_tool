import { Injectable } from '@angular/core';
import { isTauri } from '../core/platform';
import { nativePickOne } from '../core/native-dialog';

/** Where an export goes: the native save dialog in Tauri; a default name in a browser. Replaceable in tests. */
@Injectable({ providedIn: 'root' })
export class ExportTarget {
  /** The chosen `.zip` path, or null when cancelled. */
  async pickZip(defaultName: string, title = $localize`:@@diagnostics.exportTarget.title:Export diagnostics`): Promise<string | null> {
    if (!isTauri()) return defaultName;
    return nativePickOne({ kind: 'save', title, defaultName, filters: [{ name: $localize`:@@diagnostics.exportTarget.zip:Zip archive`, extensions: ['zip'] }] });
  }
}
