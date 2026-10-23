import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { ProjectService } from '../projects/project.service';
import { LifecycleService } from './lifecycle.service';

/** Right end of the title bar: project name with an unsaved dot, and the Administrator badge. */
@Component({
  selector: 'debrief-title-extras',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { style: 'display: contents' },
  template: `
    <span class="project" data-testid="project-title" [title]="projects.path() ?? notSaved">
      {{ projects.name() }}@if (projects.dirty()) {<span class="unsaved" role="img" aria-label="Unsaved changes" title="Unsaved changes" i18n-aria-label="@@lifecycle.titleExtras.unsaved" i18n-title="@@lifecycle.titleExtras.unsavedTitle">●</span>}
    </span>
    @if (life.elevated()) {
      <span class="admin" data-testid="admin-badge" title="Debrief is running with administrator rights" i18n-title="@@lifecycle.titleExtras.adminTitle" i18n="@@lifecycle.titleExtras.admin">Administrator</span>
    }
  `,
  styles: `
    .project { font-size: var(--fs-sm); color: var(--text-2); white-space: nowrap; max-width: 220px; overflow: hidden; text-overflow: ellipsis; }
    .unsaved { margin-inline-start: 6px; color: var(--warn-text); }
    .admin { font-size: var(--fs-xs); font-weight: 600; padding: 1px 8px; border-radius: 10px; background: var(--chip-bg); border: 1px solid var(--lvl-warn); color: var(--warn-text); white-space: nowrap; }
  `,
})
export class TitleExtras {
  protected readonly life = inject(LifecycleService);
  protected readonly projects = inject(ProjectService);
  protected readonly notSaved = $localize`:@@lifecycle.titleExtras.notSaved:Not saved yet`;
}
