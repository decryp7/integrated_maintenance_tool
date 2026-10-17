import { Injectable } from '@angular/core';
import { isTauri } from '../core/platform';

const FILTERS = [{ name: 'IMT settings', extensions: ['json'] }];
/** What the browser build (mock backend) uses instead of native dialogs. */
export const DEMO_SETTINGS_FILE = '/demo/imt-settings.json';

/** Native file dialogs for the Settings screen; the engine re-checks every chosen path. */
@Injectable({ providedIn: 'root' })
export class SettingsFiles {
  /* v8 ignore start: thin wrappers over the Tauri dialog plugin, exercised only in the real app */
  async pickExport(): Promise<string | null> {
    if (!isTauri()) return DEMO_SETTINGS_FILE;
    const { save } = await import('@tauri-apps/plugin-dialog');
    return save({ title: 'Export settings', defaultPath: 'imt-settings.json', filters: FILTERS });
  }

  async pickImport(): Promise<string | null> {
    if (!isTauri()) return DEMO_SETTINGS_FILE;
    const { open } = await import('@tauri-apps/plugin-dialog');
    const r = await open({ title: 'Import settings', multiple: false, directory: false, filters: FILTERS });
    return typeof r === 'string' ? r : null;
  }

  /** For `path` fields. */
  async pickPath(directory: boolean, title: string): Promise<string | null> {
    if (!isTauri()) return directory ? '/demo/folder' : '/demo/file.txt';
    const { open } = await import('@tauri-apps/plugin-dialog');
    const r = await open({ title, multiple: false, directory });
    return typeof r === 'string' ? r : null;
  }
  /* v8 ignore stop */
}
