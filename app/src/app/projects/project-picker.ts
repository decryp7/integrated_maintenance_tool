import { Injectable } from '@angular/core';
import { isTauri } from '../core/platform';

const FILTERS = [{ name: $localize`:File dialog filter|@@projects.picker.filter:IMT project`, extensions: ['imtproj'] }];

/** Native open/save dialogs for project files. In a plain browser it returns demo paths. */
@Injectable({ providedIn: 'root' })
export class ProjectPicker {
  /** Browser mode only: the path last chosen to save, offered again by "Open". */
  private browserPath = '/demo/projects/untitled.imtproj';

  /** Asks for a project to open; null when cancelled. */
  async pickOpen(): Promise<string | null> {
    if (!isTauri()) return this.browserPath;
    const { open } = await import('@tauri-apps/plugin-dialog');
    const result = await open({ multiple: false, directory: false, title: $localize`:@@projects.picker.openTitle:Open project`, filters: FILTERS });
    return typeof result === 'string' ? result : null;
  }

  /** Asks where to save a project; null when cancelled. */
  async pickSave(defaultName: string): Promise<string | null> {
    if (!isTauri()) {
      this.browserPath = `/demo/projects/${defaultName}`;
      return this.browserPath;
    }
    const { save } = await import('@tauri-apps/plugin-dialog');
    return save({ title: $localize`:@@projects.picker.saveTitle:Save project`, defaultPath: defaultName, filters: FILTERS });
  }
}
