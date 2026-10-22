import { inject, Injectable, Injector } from '@angular/core';
import { WorkspaceStore } from '../../core/workspace.store';
import { RecentService } from '../../core/recent/recent.service';
import { ProjectService } from '../../projects/project.service';
import { UnsavedChangesGuard } from '../../projects/unsaved-changes';

/** What the File ▸ Open recent items do (also used by the welcome page's list). */
@Injectable({ providedIn: 'root' })
export class RecentOpener {
  private readonly store = inject(WorkspaceStore);
  // Resolved on use: showing a list of recent items must not need the project machinery.
  private readonly injector = inject(Injector);
  private readonly recent = inject(RecentService);

  /** Opens a recent file or folder into the workspace. */
  openFile(path: string): Promise<unknown> {
    return this.store.openPaths([path]);
  }

  /** Opens a recent project, after asking about unsaved changes of the current one. */
  async openProject(path: string): Promise<void> {
    if (await this.injector.get(UnsavedChangesGuard).confirm($localize`:Completes "Save the changes before you ..."|@@projects.cmd.actionOpen:open another project`)) await this.injector.get(ProjectService).open(path);
  }

  /** Empties the recent files and projects. */
  async clear(): Promise<void> {
    await Promise.all([this.recent.clear('files'), this.recent.clear('projects')]);
  }
}
