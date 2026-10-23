import { Injectable } from '@angular/core';
import { isTauri } from '../core/platform';
import { nativePickOne } from '../core/native-dialog';

const FILTERS = [{ name: $localize`:File type in the open/save dialog@@settings.files.filter:Debrief settings`, extensions: ['json'] }];
/** What the browser build (mock backend) uses instead of native dialogs. */
export const DEMO_SETTINGS_FILE = '/demo/debrief-settings.json';

/** Native file dialogs for the Settings screen; the engine re-checks every chosen path. */
@Injectable({ providedIn: 'root' })
export class SettingsFiles {
  /* v8 ignore start: thin wrappers over the Tauri dialog plugin, exercised only in the real app */
  async pickExport(): Promise<string | null> {
    if (!isTauri()) return DEMO_SETTINGS_FILE;
    return nativePickOne({ kind: 'save', title: $localize`:@@settings.files.export.title:Export settings`, defaultName: 'debrief-settings.json', filters: FILTERS });
  }

  async pickImport(): Promise<string | null> {
    if (!isTauri()) return DEMO_SETTINGS_FILE;
    return nativePickOne({ kind: 'openFile', title: $localize`:@@settings.files.import.title:Import settings`, filters: FILTERS });
  }

  /** For `path` fields. */
  async pickPath(directory: boolean, title: string): Promise<string | null> {
    if (!isTauri()) return directory ? '/demo/folder' : '/demo/file.txt';
    return nativePickOne({ kind: directory ? 'openFolder' : 'openFile', title });
  }
  /* v8 ignore stop */
}
