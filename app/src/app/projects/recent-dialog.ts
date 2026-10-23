import { PathText } from '../shared/path-text';
import { ChangeDetectionStrategy, Component, inject, Injectable } from '@angular/core';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { formatOpenedAt } from '../core/recent/recent-date';
import { baseName } from '../tools/tool-errors';
import { ProjectService } from './project.service';
import { UnsavedChangesGuard } from './unsaved-changes';

/** Opens the list of recent projects. */
@Injectable({ providedIn: 'root' })
export class RecentProjectsDialogService {
  private readonly dialogs = inject(DialogService);
  open(): void {
    this.dialogs.open(RecentProjectsDialog, { title: $localize`:@@projects.recent.title:Recent projects`, width: 520 });
  }
}

@Component({
  selector: 'debrief-recent-projects-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [PathText],
  template: `
    @if (projects.recent().length) {
      <ul class="list" aria-label="Recent projects" i18n-aria-label="@@projects.recent.list">
        @for (p of projects.recent(); track p.path) {
          <li>
            <button type="button" class="item" (click)="open(p.path)">
              <strong>{{ p.name }}</strong>
              <debrief-path-text class="faint mono" [value]="folder(p.path)" [full]="p.path" />
              <span class="faint when" data-testid="recent-when">{{ when(p.openedAt) }}</span>
            </button>
          </li>
        }
      </ul>
    } @else {
      <p class="faint" i18n="@@projects.recent.empty">No recent projects. Save a project to see it here.</p>
    }
    <div class="foot">
      <button type="button" class="btn" [disabled]="!projects.recent().length" (click)="projects.clearRecent()" i18n="@@projects.recent.clear">Clear list</button>
      <button type="button" class="btn" (click)="ref.close()" i18n="@@projects.recent.close">Close</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 12px; }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
    .item { width: 100%; display: flex; flex-direction: column; gap: 2px; align-items: flex-start; padding: 6px 10px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); color: var(--text); cursor: pointer; text-align: start; min-width: 0; }
    .item:hover { background: var(--bg-hover); }
    .item .when { font-size: var(--fs-xs); }
    .item .ellipsis { max-width: 100%; font-size: var(--fs-xs); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
    p { margin: 0; }
  `,
})
export class RecentProjectsDialog {
  protected readonly ref = inject(DIALOG_REF);
  protected readonly projects = inject(ProjectService);
  private readonly guard = inject(UnsavedChangesGuard);

  protected when(openedAt: number): string {
    return formatOpenedAt(openedAt);
  }

  protected folder(path: string): string {
    return path.slice(0, path.length - baseName(path).length) || path;
  }

  protected async open(path: string): Promise<void> {
    this.ref.close();
    if (await this.guard.confirm($localize`:Completes "Save the changes before you ..."|@@projects.cmd.actionOpen:open another project`)) await this.projects.open(path);
  }
}
