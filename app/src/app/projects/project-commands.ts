import { inject, Injectable } from '@angular/core';
import { DebriefCommand } from '../extensions/tokens';
import { ProjectService } from './project.service';
import { RecentProjectsDialogService } from './recent-dialog';
import { RECENT_PROJECTS } from './recent-projects';
import { UnsavedChangesGuard } from './unsaved-changes';

/** File menu commands for projects. */
@Injectable({ providedIn: 'root' })
export class ProjectCommands {
  private readonly projects = inject(ProjectService);
  private readonly recent = inject(RecentProjectsDialogService);
  private readonly guard = inject(UnsavedChangesGuard);
  private readonly recentList = inject(RECENT_PROJECTS);

  readonly commands: DebriefCommand[] = [
    { id: 'project.new', title: $localize`:@@projects.cmd.new:New project`, category: $localize`:Command palette category|@@projects.cmd.fileCategory:File`, run: () => this.newProject() },
    { id: 'project.open', title: $localize`:@@projects.cmd.open:Open project…`, category: $localize`:Command palette category|@@projects.cmd.fileCategory:File`, run: () => this.openProject() },
    { id: 'project.recent', title: $localize`:@@projects.cmd.recent:Open recent project…`, category: $localize`:Command palette category|@@projects.cmd.fileCategory:File`, run: () => this.recent.open() },
    {
      id: 'project.openLast',
      title: $localize`:@@projects.cmd.openLast:Open last project`,
      category: $localize`:Command palette category|@@projects.cmd.fileCategory:File`,
      when: () => this.recentList.list().length > 0,
      run: () => this.openLast(),
    },
    { id: 'project.save', title: $localize`:@@projects.cmd.save:Save project`, category: $localize`:Command palette category|@@projects.cmd.fileCategory:File`, keybinding: 'Mod+S', run: async () => void (await this.projects.save()) },
    { id: 'project.saveAs', title: $localize`:@@projects.cmd.saveAs:Save project as…`, category: $localize`:Command palette category|@@projects.cmd.fileCategory:File`, keybinding: 'Mod+Shift+S', run: async () => void (await this.projects.saveAs()) },
  ];

  /** New and Open replace the current project, so they ask first when it has unsaved changes. */
  private async newProject(): Promise<void> {
    if (await this.guard.confirm($localize`:Completes "Save the changes before you ..."|@@projects.cmd.actionNew:start a new project`)) await this.projects.newProject();
  }

  /** Opens the most recently used project (the head of the recent list); a no-op when there is none. */
  async openLast(): Promise<void> {
    const last = this.recentList.list()[0];
    if (!last) return;
    if (await this.guard.confirm($localize`:Completes "Save the changes before you ..."|@@projects.cmd.actionOpen:open another project`)) await this.projects.open(last.path);
  }

  private async openProject(): Promise<void> {
    if (await this.guard.confirm($localize`:Completes "Save the changes before you ..."|@@projects.cmd.actionOpen:open another project`)) await this.projects.openFromDialog();
  }
}
