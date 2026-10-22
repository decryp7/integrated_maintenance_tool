import { formatExactBytes } from '../../shared/format/units';
import { ChangeDetectionStrategy, Component, computed, effect, inject, input, untracked } from '@angular/core';
import { HeapFinding, HeapFindingView } from '../../core/api.types';
import { AnalysisState } from '../../analysis/analysis-state';
import { AsyncResource } from '../../analysis/async-resource';
import { SeverityBadge } from '../../live/severity-badge';
import { CapturesStore } from '../captures.store';
import { ProfilingBackend } from '../profiling-backend';
import { formatBytes } from '../profiling-format';
import { PROFILING_STYLES } from '../profiling-styles';
import { byTaken } from './heap-diff';
import { HeapSelection, HeapTab } from './heap-selection';

const VIEW_TAB: Record<HeapFindingView, HeapTab> = { types: 'types', dominators: 'dominators', paths: 'instances', diff: 'diff' };

const ACTION: Record<HeapFindingView, string> = {
  types: $localize`:@@profiling.heapFindings.action.types:Show the type`,
  dominators: $localize`:@@profiling.heapFindings.action.dominators:Show what it retains`,
  paths: $localize`:@@profiling.heapFindings.action.paths:Show paths to roots`,
  diff: $localize`:@@profiling.heapFindings.action.diff:Compare snapshots`,
};

/** The engine's findings for a heap snapshot (growth against the snapshot taken before it), each linking to its evidence. */
@Component({
  selector: 'debrief-heap-findings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, SeverityBadge],
  template: `
    <debrief-analysis-state [loading]="res.loading()" [error]="res.error()" [hasData]="!!res.data()" [empty]="res.data()?.length === 0" emptyText="No findings: nothing in this snapshot crosses a threshold." i18n-emptyText="@@profiling.heapFindings.empty" (retry)="res.retry()">
      <ul class="list body" aria-label="Memory findings" i18n-aria-label="@@profiling.heapFindings.listLabel">
        @for (f of res.data() ?? []; track $index) {
          <li class="finding" [attr.data-kind]="f.kind">
            <div class="head">
              <debrief-severity-badge [severity]="f.severity" />
              <strong class="title">{{ f.title }}</strong>
              @if (f.valueBytes !== null) { <span class="mono muted" [title]="exact(f.valueBytes)">{{ bytes(f.valueBytes) }}</span> }
            </div>
            <p class="detail">{{ f.detail }}</p>
            <button class="btn" type="button" (click)="open(f)" [attr.aria-label]="actionLabel(f)">{{ action(f) }}</button>
          </li>
        }
      </ul>
    </debrief-analysis-state>
  `,
  styles: [
    PROFILING_STYLES,
    `
      .finding { border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); padding: 8px 10px; margin: 0 12px 8px; }
      .head { display: flex; align-items: center; gap: 8px; }
      .title { color: var(--text); }
      .detail { margin: 4px 0 6px; color: var(--text-2); }
    `,
  ],
})
export class HeapFindings {
  readonly snapshotId = input.required<number>();
  private readonly backend = inject(ProfilingBackend);
  private readonly captures = inject(CapturesStore);
  private readonly sel = inject(HeapSelection);
  protected readonly res = new AsyncResource<HeapFinding[]>();
  protected readonly bytes = formatBytes;
  protected readonly exact = formatExactBytes;
  /** The snapshot taken just before this one (growth is measured against it). */
  private readonly baselineId = computed(() => {
    const ordered = byTaken(this.captures.heaps());
    const i = ordered.findIndex((h) => h.id === this.snapshotId());
    return i > 0 ? (ordered[i - 1]?.id ?? null) : null;
  });

  constructor() {
    effect(() => {
      const snapshotId = this.snapshotId();
      const baselineId = this.baselineId();
      untracked(() => void this.res.load(() => this.backend.heapFindings({ snapshotId, baselineId })));
    });
  }

  protected action(f: HeapFinding): string {
    return ACTION[f.view];
  }

  protected actionLabel(f: HeapFinding): string {
    return $localize`:@@profiling.heapFindings.actionLabel:${ACTION[f.view]}:action: for ${f.title}:title:`;
  }

  protected open(f: HeapFinding): void {
    if (f.typeName && (f.view === 'types' || f.view === 'paths')) {
      this.sel.showInstances(f.typeName);
      return;
    }
    this.sel.tab.set(VIEW_TAB[f.view]);
  }
}
