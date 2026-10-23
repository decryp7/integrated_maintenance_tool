import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { HeapSummary } from '../../core/api.types';
import { formatCount } from '../../core/format';
import { CapturesStore } from '../captures.store';
import { ProfileViewState } from '../profile-view-state.store';
import { formatBytes } from '../profiling-format';
import { ActiveProfile } from '../profiling-navigator';
import { CaptureNotes, NoCapture } from '../profiling-shared';
import { PROFILING_STYLES } from '../profiling-styles';
import { HeapDiff } from './heap-diff';
import { HeapDominators } from './heap-dominators';
import { HeapFindings } from './heap-findings';
import { HeapGrowth } from './heap-growth';
import { HeapInstances } from './heap-instances';
import { HEAP_TABS, HeapSelection, HeapTab } from './heap-selection';
import { HeapTypes } from './heap-types';

/** Heap snapshots: types, snapshot diff, growth, dominator tree, instances and paths to GC roots. */
@Component({
  selector: 'debrief-heap-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [NoCapture, CaptureNotes, HeapFindings, HeapTypes, HeapDiff, HeapGrowth, HeapDominators, HeapInstances],
  providers: [HeapSelection],
  template: `
    @if (active.heap(); as h) {
      <div class="bar">
        <span class="section-title" i18n="@@profiling.heap.title">Memory</span>
        <label class="inline"><ng-container i18n="@@profiling.heap.snapshot">Snapshot</ng-container>&ngsp;
          <select aria-label="Heap snapshot" i18n-aria-label="@@profiling.heap.snapshotLabel" (change)="pick($event)">
            @for (c of captures.heaps(); track c.id) { <option [value]="c.id" [selected]="c.id === h.id">{{ c.name }}</option> }
          </select>
        </label>
        <span class="muted">{{ summary(h) }}</span>
      </div>
      <debrief-capture-notes [notes]="h.notes" [name]="h.name" />
      <div class="tabs" role="tablist" aria-label="Memory views" i18n-aria-label="@@profiling.heap.tabsLabel" (keydown)="onTabKey($event)">
        @for (t of tabs; track t.id) {
          <button type="button" role="tab" [id]="'heap-tab-' + t.id" [attr.aria-selected]="sel.tab() === t.id" [attr.aria-controls]="'heap-panel'" [attr.tabindex]="sel.tab() === t.id ? 0 : -1" [disabled]="needsGraph(t.id) && !h.hasGraph" (click)="sel.tab.set(t.id)">{{ t.label }}</button>
        }
      </div>
      <div class="panel" role="tabpanel" id="heap-panel" [attr.aria-labelledby]="'heap-tab-' + sel.tab()">
        @switch (sel.tab()) {
          @case ('findings') { <debrief-heap-findings [snapshotId]="h.id" /> }
          @case ('types') { <debrief-heap-types [snapshotId]="h.id" /> }
          @case ('diff') { <debrief-heap-diff /> }
          @case ('growth') { <debrief-heap-growth /> }
          @case ('dominators') { <debrief-heap-dominators [snapshotId]="h.id" /> }
          @case ('instances') { <debrief-heap-instances [snapshotId]="h.id" /> }
        }
      </div>
    } @else {
      <debrief-no-capture text="Open a heap snapshot (.heapsnapshot) to analyse memory." i18n-text="@@profiling.heap.noCapture" />
    }
  `,
  styles: [
    PROFILING_STYLES,
    `
      .tabs { display: flex; gap: 2px; padding: 0 12px; border-bottom: 1px solid var(--border); flex: none; }
      .tabs button { border: 0; border-bottom: 2px solid transparent; background: transparent; color: var(--muted); font: inherit; padding: 6px 10px; cursor: pointer; }
      .tabs button[aria-selected='true'] { color: var(--text); border-bottom-color: var(--accent); }
      .tabs button:disabled { opacity: 0.45; cursor: default; }
      .tabs button:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
      .panel { flex: 1; min-height: 0; display: flex; flex-direction: column; }
      .panel > * { flex: 1; min-height: 0; display: flex; flex-direction: column; }
    `,
  ],
})
export class HeapView {
  protected readonly active = inject(ActiveProfile);
  protected readonly captures = inject(CapturesStore);
  protected readonly sel = inject(HeapSelection);
  private readonly state = inject(ProfileViewState);
  protected readonly tabs = HEAP_TABS;
  protected readonly count = formatCount;
  protected readonly bytes = formatBytes;

  protected summary(h: HeapSummary): string {
    const objects = this.count(h.objectCount);
    const types = this.count(h.typeCount);
    const bytes = this.bytes(h.totalBytes);
    return h.hasGraph
      ? $localize`:@@profiling.heap.summaryGraph:${objects}:objects: objects · ${types}:types: types · ${bytes}:bytes:`
      : $localize`:@@profiling.heap.summaryNoGraph:${objects}:objects: objects · ${types}:types: types · ${bytes}:bytes: · no object graph (types only)`;
  }

  protected needsGraph(tab: HeapTab): boolean {
    return tab === 'dominators';
  }

  protected pick(e: Event): void {
    this.state.setHeap(Number((e.target as HTMLSelectElement).value));
  }

  /** Left/Right move between tabs (automatic activation). */
  protected onTabKey(e: KeyboardEvent): void {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    e.preventDefault();
    const hasGraph = !!this.active.heap()?.hasGraph;
    const ids = HEAP_TABS.map((t) => t.id).filter((id) => hasGraph || !this.needsGraph(id));
    const i = Math.max(0, ids.indexOf(this.sel.tab()));
    const next = ids[(i + (e.key === 'ArrowRight' ? 1 : ids.length - 1)) % ids.length];
    if (next === undefined) return;
    this.sel.tab.set(next);
    const list = e.currentTarget as HTMLElement;
    queueMicrotask(() => list.querySelector<HTMLElement>(`#heap-tab-${next}`)?.focus());
  }
}
