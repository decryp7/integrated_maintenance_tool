import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { Query } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { CommandRegistry } from '../extensions/command-registry';
import { Icon } from '../shared/icon';

/** What a view is empty or broken because of, as far as the shell can tell. */
export type EmptyCause = 'no-files' | 'filtered' | 'no-range' | 'none';

/** Why the current query may hide everything: true when anything narrows the events. */
export function isFiltered(q: Query): boolean {
  return !!(q.text || q.levels.length || q.filters.length || q.groups.length || q.markers.length || (q as Query & { ql?: string }).ql);
}

/** The most useful thing to suggest: open files first, then widen the filters, then the time range. */
export function emptyCause(hasFiles: boolean, query: Query): EmptyCause {
  if (!hasFiles) return 'no-files';
  if (isFiltered(query)) return 'filtered';
  if (query.from !== null || query.to !== null) return 'no-range';
  return 'none';
}

/**
 * The next step for an empty or failed view, decided from the workspace: "Open folder…" (and the demo)
 * when nothing is open, "Clear filters" when filters may hide everything, "Show the whole time range"
 * when zoomed in. Renders nothing when none applies. Views add their own specific actions next to it.
 */
@Component({
  selector: 'imt-next-steps',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    @switch (cause()) {
      @case ('no-files') {
        @if (!hideOpen()) {
          <span class="why" i18n="@@ui.nextSteps.nothingOpen">Nothing is open yet.</span>
          <button type="button" class="btn primary" (click)="store.pickAndOpen(true)" i18n="@@ui.nextSteps.openFolder"><imt-icon name="folder" [size]="13" /> Open folder…</button>
          <button type="button" class="btn" (click)="store.pickAndOpen(false)" i18n="@@ui.nextSteps.openFiles"><imt-icon name="file" [size]="13" /> Open files…</button>
          @if (demo()) {
            <button type="button" class="btn ghost" (click)="runDemo()" i18n="@@ui.nextSteps.tryDemo">Try the demo</button>
          }
        }
      }
      @case ('filtered') {
        <span class="why" i18n="@@ui.nextSteps.filtered">Filters may be hiding events.</span>
        <button type="button" class="btn" (click)="store.clearQuery()" i18n="@@ui.nextSteps.clearFilters"><imt-icon name="close" [size]="13" /> Clear all filters</button>
      }
      @case ('no-range') {
        <span class="why" i18n="@@ui.nextSteps.partialRange">Only part of the time range is shown.</span>
        <button type="button" class="btn" (click)="store.resetRange()" i18n="@@ui.nextSteps.showWholeRange"><imt-icon name="zoomOut" [size]="13" /> Show the whole time range</button>
      }
    }
  `,
  styles: `
    :host { display: flex; flex-wrap: wrap; align-items: center; justify-content: center; gap: 8px; }
    .why { flex-basis: 100%; text-align: center; color: var(--muted); font-size: 12px; }
    .btn { gap: 6px; }
  `,
})
export class NextSteps {
  protected readonly store = inject(WorkspaceStore);
  private readonly commands = inject(CommandRegistry);
  /** The view offers its own way to open files (e.g. the sidebar): skip the open buttons. */
  readonly hideOpen = input(false);

  protected readonly cause = computed(() => emptyCause((this.store.summary()?.sources.length ?? 0) > 0, this.store.query()));
  protected readonly demo = computed(() => this.commands.available().some((c) => c.id === 'help.demo'));

  protected runDemo(): void {
    void this.commands.run('help.demo');
  }
}
