import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { formatCount } from '../core/format';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { plural } from '../i18n/locale';
import { Icon } from '../shared/icon';
import { PROBLEMS_VIEW } from './problems-model';
import { ProblemsStore } from './problems.store';

/** The status-bar badge: how many import problems there are; it opens the Problems pane. */
@Component({
  selector: 'imt-problems-badge',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    @if (store.loaded()) {
      <button type="button" class="pill" data-testid="status-problems" [class.on]="store.counts().total > 0" [title]="label()" [attr.aria-label]="label()" (click)="ctx.showView(view)">
        <imt-icon name="alert" [size]="12" />{{ fmt(store.counts().total) }}
      </button>
    }
  `,
  styles: `
    :host { display: inline-flex; }
    .pill { display: inline-flex; align-items: center; gap: 4px; height: 18px; padding: 0 7px; border-radius: 9px; border: 1px solid transparent; background: transparent; color: inherit; cursor: pointer; font-size: var(--fs-xs); font-weight: 600; }
    .pill:hover, .pill.on { border-color: currentColor; }
    .pill:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
  `,
})
export class ProblemsBadge {
  protected readonly store = inject(ProblemsStore);
  protected readonly ctx = inject(COMMAND_CONTEXT);
  protected readonly view = PROBLEMS_VIEW;
  protected readonly fmt = formatCount;

  protected readonly label = computed(() => {
    const n = this.store.counts().total;
    return plural(n, {
      '=0': $localize`:@@problems.status.none:0 problems: show the Problems pane`,
      one: $localize`:@@problems.status.one:${formatCount(n)}:count: problem in the opened files: show the Problems pane`,
      other: $localize`:@@problems.status.other:${formatCount(n)}:count: problems in the opened files: show the Problems pane`,
    });
  });
}
