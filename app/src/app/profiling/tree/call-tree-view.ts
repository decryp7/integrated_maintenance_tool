import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, Injector, input, untracked, viewChild } from '@angular/core';
import { RovingFocusNavigation } from '../../shared/list-navigation';
import { TreeDirection } from '../../core/api.types';
import { AnalysisState } from '../../analysis/analysis-state';
import { ProfileViewState } from '../profile-view-state';
import { formatMetric, formatPct } from '../profiling-format';
import { ActiveProfile, ProfilingNavigator } from '../profiling-navigator';
import { CaptureNotes, MeasurementLegend, MeasurementTag, NoCapture, ProfileBar } from '../profiling-shared';
import { PROFILING_STYLES } from '../profiling-styles';
import { ROOT_KEY, TreeRow, treeKey } from './tree-model';
import { TreeStore } from './tree.store';

/**
 * Call tree (top-down) or inverted tree (bottom-up) as an accessible treegrid: lazily paged
 * children, expand hottest path, roving focus with the arrow keys (Right expands, Left collapses or
 * goes to the parent, Enter shows callers and callees, * expands the hottest path below).
 */
@Component({
  selector: 'imt-profile-tree',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, ProfileBar, MeasurementTag, MeasurementLegend, CaptureNotes, NoCapture],
  providers: [TreeStore],
  template: `
    @if (active.profile(); as p) {
      <imt-profile-bar />
      <imt-capture-notes [notes]="p.notes" [name]="p.name" />
      <div class="bar">
        <span class="section-title">{{ title() }}</span>
        <button class="btn" type="button" [disabled]="store.busy()" (click)="hotPath()" i18n="@@profiling.tree.hotPath">Expand hottest path</button>
        <button class="btn" type="button" [disabled]="!store.expanded().length" (click)="collapseAll()" i18n="@@profiling.tree.collapseAll">Collapse all</button>
        @if (store.notice(); as n) { <span class="muted" role="status">{{ n }}</span> }
        <span class="spacer"></span>
        <imt-measurement-legend />
      </div>
      <imt-analysis-state [loading]="store.root().loading" [error]="store.root().error" [hasData]="store.root().children !== null" [empty]="store.root().children?.length === 0" emptyText="No samples match this view." i18n-emptyText="@@profiling.tree.empty" (retry)="store.retry('')">
        <div class="body tight" tabindex="0" role="region" aria-label="Call tree rows" i18n-aria-label="@@profiling.tree.rowsRegion">
          <table class="grid" role="treegrid" [attr.aria-label]="title()" (keydown)="onKey($event)" #grid>
            <thead>
              <tr role="row">
                <th role="columnheader" scope="col" i18n="@@profiling.tree.col.function">Function</th>
                <th role="columnheader" scope="col" class="num"><ng-container i18n="@@profiling.tree.col.totalPct">Total %</ng-container>&ngsp;@if (store.measurement(); as m) { <imt-measurement [m]="m" /> }</th>
                <th role="columnheader" scope="col" class="num"><ng-container i18n="@@profiling.tree.col.total">Total</ng-container>&ngsp;@if (store.measurement(); as m) { <imt-measurement [m]="m" /> }</th>
                <th role="columnheader" scope="col" class="num"><ng-container i18n="@@profiling.tree.col.selfPct">Self %</ng-container>&ngsp;@if (store.measurement(); as m) { <imt-measurement [m]="m" /> }</th>
                <th role="columnheader" scope="col" class="num"><ng-container i18n="@@profiling.tree.col.self">Self</ng-container>&ngsp;@if (store.measurement(); as m) { <imt-measurement [m]="m" /> }</th>
              </tr>
            </thead>
            <tbody>
              @for (r of store.rows(); track r.key; let i = $index) {
                @if (r.kind === 'node') {
                  <tr
                    role="row"
                    [attr.aria-level]="r.level"
                    [attr.aria-posinset]="r.posinset"
                    [attr.aria-setsize]="r.setsize"
                    [attr.aria-expanded]="r.entry.node.hasChildren ? r.expanded : null"
                    [attr.aria-selected]="store.selected() === r.key"
                    [attr.tabindex]="focusIndex() === i ? 0 : -1"
                    [attr.data-key]="r.key"
                    [class.sel]="store.selected() === r.key"
                    [class.framework]="r.entry.node.frame.isFramework"
                    (click)="focusRow(i); store.select(r.key)"
                    (dblclick)="activate(r)"
                  >
                    <td role="gridcell" class="fn"><span class="fnwrap" [style.padding-left.px]="(r.level - 1) * 14">
                      @if (r.entry.node.hasChildren) {
                        <span class="tw" aria-hidden="true" (click)="$event.stopPropagation(); store.toggle(r.key)">{{ r.expanded ? '▾' : '▸' }}</span>
                      } @else {
                        <span class="tw" aria-hidden="true"></span>
                      }
                      <span class="ellipsis" [title]="r.entry.node.frame.name + (r.entry.node.frame.module ? ' (' + r.entry.node.frame.module + ')' : '')">{{ r.entry.node.frame.name }}</span>
                      @if (r.entry.node.frame.isFramework) { <span class="tag" i18n="@@profiling.tree.framework">framework</span> }
                    </span></td>
                    <td role="gridcell" class="num"><span class="bar-cell"><span class="meter" aria-hidden="true"><span [style.width.%]="r.entry.node.totalPct"></span></span>{{ pct(r.entry.node.totalPct) }}</span></td>
                    <td role="gridcell" class="num">{{ value(r.entry.node.total) }}</td>
                    <td role="gridcell" class="num">{{ pct(r.entry.node.selfPct) }}</td>
                    <td role="gridcell" class="num">{{ value(r.entry.node.self) }}</td>
                  </tr>
                } @else if (r.kind === 'more') {
                  <tr role="row" [attr.aria-level]="r.level" [attr.tabindex]="focusIndex() === i ? 0 : -1" (click)="focusRow(i); store.loadMore(r.parent)">
                    <td role="gridcell" colspan="5" [style.padding-left.px]="22 + (r.level - 1) * 14"><span class="link" i18n="@@profiling.tree.loadMore">Load {{ r.remaining }} more…</span></td>
                  </tr>
                } @else {
                  <tr role="row" [attr.aria-level]="r.level" [attr.tabindex]="focusIndex() === i ? 0 : -1">
                    <td role="gridcell" colspan="5" [style.padding-left.px]="22 + (r.level - 1) * 14">
                      @if (r.error) {
                        <span class="err" role="alert">{{ r.error }}</span> <button class="mini" type="button" (click)="store.retry(r.parent)" i18n="@@profiling.tree.retry">Retry</button>
                      } @else {
                        <span class="muted" i18n="@@profiling.tree.loading">Loading…</span>
                      }
                    </td>
                  </tr>
                }
              }
            </tbody>
          </table>
        </div>
      </imt-analysis-state>
    } @else {
      <imt-no-capture text="Open a performance capture to see its call tree." i18n-text="@@profiling.tree.noCapture" />
    }
  `,
  styles: [
    PROFILING_STYLES,
    `
      .tight { padding: 0; }
      table.grid td, table.grid th { padding-top: 2px; padding-bottom: 2px; }
      tr[role='row'][tabindex]:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
      td.fn { max-width: 640px; }
      .fnwrap { display: flex; align-items: center; gap: 4px; min-width: 0; }
      .tw { width: 12px; flex: none; cursor: pointer; color: var(--muted); }
      tr.framework td.fn .ellipsis { color: var(--muted); }
      .err { color: var(--danger-text); }
    `,
  ],
})
export class ProfileTree {
  readonly direction = input<TreeDirection>('topDown');
  protected readonly store = inject(TreeStore);
  protected readonly active = inject(ActiveProfile);
  private readonly state = inject(ProfileViewState);
  private readonly nav = inject(ProfilingNavigator);
  private readonly grid = viewChild<ElementRef<HTMLTableElement>>('grid');
  /** The row with the tab stop (CDK FocusKeyManager: the row takes focus when it moves). */
  private readonly rowNav = new RovingFocusNavigation(
    computed(() => this.store.rows().length),
    inject(Injector),
    (i) => queueMicrotask(() => this.grid()?.nativeElement.querySelectorAll<HTMLElement>('tbody tr')[i]?.focus()),
    { selectFirst: true, homeAndEnd: true },
  );
  protected readonly focusIndex = this.rowNav.active;
  protected readonly pct = formatPct;
  protected readonly title = computed(() => (this.direction() === 'bottomUp' ? $localize`:@@profiling.tree.titleInverted:Inverted tree (bottom-up)` : $localize`:@@profiling.tree.titleTopDown:Call tree (top-down)`));

  constructor() {
    effect(() => {
      const p = this.active.profile();
      const metric = this.active.metric();
      const direction = this.direction();
      const view = this.state.view();
      untracked(() => {
        this.store.setDirection(direction);
        this.rowNav.sync(0);
        void this.store.setQuery(p && metric ? { profileId: p.id, view, metric } : null).then(() => this.applyReveal());
      });
    });
    effect(() => {
      const r = this.state.reveal();
      if (r) untracked(() => void this.applyReveal());
    });
  }

  /** Follows a pending reveal for this tree's direction (from a finding or a hotspot row). */
  private async applyReveal(): Promise<void> {
    const r = this.state.reveal();
    if (!r || r.direction !== this.direction() || !this.active.profile()) return;
    if (await this.store.reveal(r.path)) {
      const i = this.store.rows().findIndex((x) => x.key === this.store.selected());
      if (i >= 0) this.focusRow(i, false);
      if (r.direction === 'bottomUp') await this.store.expandHotPath(this.store.selected() ?? ROOT_KEY);
    }
  }

  protected value(v: Partial<Record<string, number>>): string {
    const m = this.active.metric();
    return m ? formatMetric(v[m], m, this.active.profile()?.metrics) : '—';
  }

  protected hotPath(): void {
    const sel = this.store.selected();
    void this.store.expandHotPath(sel ?? ROOT_KEY).then(() => {
      const i = this.store.rows().findIndex((x) => x.key === this.store.selected());
      if (i >= 0) this.focusRow(i, false);
    });
  }

  protected collapseAll(): void {
    for (const k of this.store.expanded()) this.store.collapse(k);
    this.rowNav.sync(0);
  }

  protected activate(r: TreeRow): void {
    if (r.kind === 'node') this.nav.butterfly(r.entry.node.frame);
  }

  protected focusRow(i: number, focus = true): void {
    if (focus) this.rowNav.setActive(i);
    else this.rowNav.sync(i);
  }

  protected onKey(e: KeyboardEvent): void {
    const rows = this.store.rows();
    // Up, Down, Home and End move the tab stop through the CDK key manager; the tree keys (expand, collapse, parent) follow the model.
    if (this.rowNav.handle(e)) {
      e.preventDefault();
      const r = rows[this.focusIndex()];
      if (r?.kind === 'node') this.store.select(r.key);
      return;
    }
    const action = treeKey(rows, this.focusIndex(), e.key);
    if (!action) return;
    e.preventDefault();
    switch (action.type) {
      case 'focus': {
        this.focusRow(action.index);
        const r = rows[action.index];
        if (r?.kind === 'node') this.store.select(r.key);
        break;
      }
      case 'expand':
        void this.store.expand(action.key);
        break;
      case 'collapse':
        this.store.collapse(action.key);
        break;
      case 'hotPath':
        this.store.select(action.key);
        this.hotPath();
        break;
      case 'activate': {
        const r = rows[action.index];
        if (!r) break;
        if (r.kind === 'more') void this.store.loadMore(r.parent);
        else this.activate(r);
        break;
      }
    }
  }
}

/** The "Call tree" dock view. */
@Component({
  selector: 'imt-call-tree-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ProfileTree],
  template: `<imt-profile-tree direction="topDown" />`,
  styles: `:host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; } imt-profile-tree { flex: 1; min-height: 0; }`,
})
export class CallTreeView {}

/** The "Inverted tree" dock view. */
@Component({
  selector: 'imt-inverted-tree-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ProfileTree],
  template: `<imt-profile-tree direction="bottomUp" />`,
  styles: `:host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; } imt-profile-tree { flex: 1; min-height: 0; }`,
})
export class InvertedTreeView {}
