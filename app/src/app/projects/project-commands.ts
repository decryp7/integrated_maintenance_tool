import { inject, Injectable } from '@angular/core';
import { ImtCommand } from '../extensions/tokens';
import { ProjectService } from './project.service';
import { RecentProjectsDialogService } from './recent-dialog';
import { UnsavedChangesGuard } from './unsaved-changes';

/** File menu commands for projects. */
@Injectable({ providedIn: 'root' })
export class ProjectCommands {
  private readonly projects = inject(ProjectService);
  private readonly recent = inject(RecentProjectsDialogService);
  private readonly guard = inject(UnsavedChangesGuard);

  readonly commands: ImtCommand[] = [
    { id: 'project.new', title: 'New project', category: 'File', run: () => this.newProject() },
    { id: 'project.open', title: 'Open project…', category: 'File', run: () => this.openProject() },
    { id: 'project.recent', title: 'Open recent project…', category: 'File', run: () => this.recent.open() },
    { id: 'project.save', title: 'Save project', category: 'File', keybinding: 'Mod+S', run: async () => void (await this.projects.save()) },
    { id: 'project.saveAs', title: 'Save project as…', category: 'File', keybinding: 'Mod+Shift+S', run: async () => void (await this.projects.saveAs()) },
  ];

  /** New and Open replace the current project, so they ask first when it has unsaved changes. */
  private async newProject(): Promise<void> {
    if (await this.guard.confirm('start a new project')) await this.projects.newProject();
  }

  private async openProject(): Promise<void> {
    if (await this.guard.confirm('open another project')) await this.projects.openFromDialog();
  }
}
