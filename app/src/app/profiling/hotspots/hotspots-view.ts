import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, signal, untracked, viewChild } from '@angular/core';
import { HotspotRow, ProfileGroupBy } from '../../core/api.types';
import { formatCount } from '../../core/format';
import { AnalysisState } from '../../analysis/analysis-state';
import { ProfileViewState } from '../profile-view-state';
import { ActiveProfile, ProfilingNavigator } from '../profiling-navigator';
import { CaptureNotes, MeasurementLegend, MeasurementTag, NoCapture, ProfileBar } from '../profiling-shared';
import { PROFILING_STYLES } from '../profiling-styles';
import { GROUP_LABELS, HotspotColumn, hotspotColumns, sameSort } from './hotspot-columns';
import { HotspotsStore } from './hotspots.store';

export const ROW_H = 26;
const OVERSCAN = 8;
const FALLBACK_H = 420;

/** Rows to render for a scroll position: first..last (exclusive) plus spacer heights. */
export function rowWindow(total: number, scrollTop: number, height: number, rowH = ROW_H, overscan = OVERSCAN): { first: number; last: number; before: number; after: number } {
  const first = Math.min(total, Math.max(0, Math.floor(scrollTop / rowH) - overscan));
  const last = Math.min(total, Math.ceil((scrollTop + height) / rowH) + overscan);
  return { first, last, before: first * rowH, after: Math.max(0, (total - last) * rowH) };
}

/**
 * Hotspots: functions (or types, namespaces, modules, threads) with self/total values, paged and
 * sorted by the engine. Virtualised: only the rows in view are in the DOM. Each column header
 * tells whether its values are sampled estimates (≈) or exact (=).
 */
@Component({
  selector: 'imt-hotspots-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, ProfileBar, MeasurementTag, MeasurementLegend, CaptureNotes, NoCapture],
  providers: [HotspotsStore],
  template: `
    @if (active.profile(); as p) {
      <imt-profile-bar />
      <imt-capture-notes [notes]="p.notes" [name]="p.name" />
      <div class="bar">
        <span class="section-title">Hotspots</span>
        <label class="inline">Group by
          <select aria-label="Group by" (change)="setGroupBy($event)">
            @for (g of groups; track g) { <option [value]="g" [selected]="g === store.groupBy()">{{ groupLabel(g) }}</option> }
          </select>
        </label>
        <input type="search" placeholder="Search…" aria-label="Search hotspots" [value]="store.search()" (input)="onSearch($event)" />
        @if (store.meta(); as m) {
          <span class="muted" data-testid="hotspots-count">{{ count(m.totalRows) }} {{ groupLabel(store.groupBy()).toLowerCase() }}{{ m.totalRows === 1 ? '' : 's' }} · {{ count(m.samplesInView) }} samples in view</span>
        }
        <span class="spacer"></span>
        <imt-measurement-legend />
      </div>
      <imt-analysis-state [loading]="store.loading()" [error]="store.error()" [hasData]="!!store.meta()" [empty]="store.meta()?.totalRows === 0" emptyText="No samples match this view (range, threads or search)." (retry)="store.retry()">
        <div class="viewport" #viewport tabindex="0" role="region" aria-label="Hotspot rows" (scroll)="onScroll($event)">
          <table class="grid" [attr.aria-rowcount]="(store.meta()?.totalRows ?? 0) + 1" aria-label="Hotspots">
            <thead>
              <tr aria-rowindex="1">
                @for (c of columns(); track c.id) {
                  <th scope="col" [class.num]="c.numeric" [attr.aria-sort]="ariaSort(c)" [title]="c.help + (c.measurement ? ' — ' + measurementText(c) : '')">
                    @if (c.sort) {
                      <button type="button" (click)="store.sortBy(c.sort)">{{ c.label }}<span aria-hidden="true">{{ sortArrow(c) }}</span></button>
                    } @else {
                      <span>{{ c.label }}</span>
                    }
                    @if (c.measurement; as m) { <imt-measurement [m]="m" /> }
                  </th>
                }
                <th scope="col"><span class="sr">Actions</span></th>
              </tr>
            </thead>
            <tbody (keydown)="onKey($event)">
              @if (win().before) { <tr class="spacer" aria-hidden="true" [style.height.px]="win().before"><td [attr.colspan]="columns().length + 1"></td></tr> }
              @for (i of indices(); track i) {
                @let r = rowAt(i);
                <tr [attr.aria-rowindex]="i + 2" [class.sel]="store.selected() === i" [style.height.px]="rowH" [attr.data-index]="i">
                  @if (r) {
                    @for (c of columns(); track c.id) {
                      @if (c.id === 'name') {
                        <td class="name"><button class="link ellipsis" type="button" [attr.data-row]="i" [title]="r.name + (r.module ? ' (' + r.module + ')' : '')" (focus)="store.select(i)" (click)="primary(r, i)">{{ r.name }}</button></td>
                      } @else if (c.pct) {
                        <td class="num"><span class="bar-cell"><span class="meter" aria-hidden="true"><span [style.width.%]="c.pct === 'self' ? r.selfPct : r.totalPct"></span></span>{{ c.value(r) }}</span></td>
                      } @else {
                        <td class="num">{{ c.value(r) }}</td>
                      }
                    }
                    <td>
                      @if (r.frame) {
                        <span class="actions">
                          <button class="mini" type="button" [attr.aria-label]="'Show ' + r.name + ' in the call tree'" (click)="nav.showInTree(r, metric())">Tree</button>
                          <button class="mini" type="button" [attr.aria-label]="'Callers and callees of ' + r.name" (click)="nav.butterfly(r.frame)">Butterfly</button>
                          <button class="mini" type="button" [attr.aria-label]="'Show ' + r.name + ' in the flame graph'" (click)="nav.showInFlame(r.frame)">Flame</button>
                          <button class="mini" type="button" [attr.aria-label]="'Zoom the flame graph to ' + r.name" (click)="nav.focusInFlame(r.frame, metric())">Focus</button>
                        </span>
                      }
                    </td>
                  } @else {
                    <td class="muted" [attr.colspan]="columns().length + 1">Loading…</td>
                  }
                </tr>
              }
              @if (win().after) { <tr class="spacer" aria-hidden="true" [style.height.px]="win().after"><td [attr.colspan]="columns().length + 1"></td></tr> }
            </tbody>
          </table>
        </div>
      </imt-analysis-state>
    } @else {
      <imt-no-capture text="Open a profile to see its hotspots." />
    }
  `,
  styles: [
    PROFILING_STYLES,
    `
      .viewport { flex: 1; min-height: 0; overflow: auto; }
      table.grid td, table.grid th { padding: 0 8px; height: ${ROW_H}px; }
      table.grid th { white-space: nowrap; }
      table.grid th > * { vertical-align: middle; }
      td.name { max-width: 420px; }
      td.name .link { display: block; max-width: 100%; color: var(--text); }
      tr.spacer td { padding: 0; border: 0; height: auto; }
    `,
  ],
})
export class HotspotsView {
  protected readonly store = inject(HotspotsStore);
  protected readonly active = inject(ActiveProfile);
  protected readonly nav = inject(ProfilingNavigator);
  private readonly state = inject(ProfileViewState);
  private readonly viewport = viewChild<ElementRef<HTMLElement>>('viewport');

  protected readonly rowH = ROW_H;
  protected readonly groups: ProfileGroupBy[] = ['function', 'type', 'namespace', 'module', 'thread'];
  protected readonly count = formatCount;
  private readonly scrollTop = signal(0);
  private readonly height = signal(FALLBACK_H);
  private searchTimer: ReturnType<typeof setTimeout> | undefined;

  protected readonly metric = computed(() => this.active.metric() ?? 'samples');
  protected readonly columns = computed<HotspotColumn[]>(() => {
    const p = this.active.profile();
    return p ? hotspotColumns(p, this.metric(), GROUP_LABELS[this.store.groupBy()]) : [];
  });
  protected readonly win = computed(() => rowWindow(this.store.meta()?.totalRows ?? 0, this.scrollTop(), this.height()));
  protected readonly indices = computed(() => {
    const { first, last } = this.win();
    return Array.from({ length: last - first }, (_, k) => first + k);
  });

  constructor() {
    effect(() => {
      const p = this.active.profile();
      const query = p
        ? { profileId: p.id, view: this.state.view(), groupBy: this.store.groupBy(), metric: this.metric(), sort: this.store.sort(), search: this.store.search().trim() || null }
        : null;
      untracked(() => {
        this.scrollTop.set(0);
        this.viewport()?.nativeElement.scrollTo?.({ top: 0 });
        void this.store.setQuery(query);
      });
    });
    effect(() => {
      const { first, last } = this.win();
      untracked(() => void this.store.ensure(first, last));
    });
    const destroy = inject(DestroyRef);
    afterNextRender(() => {
      const el = this.viewport()?.nativeElement;
      if (!el) return;
      const ro = new ResizeObserver(() => this.height.set(el.clientHeight || FALLBACK_H));
      ro.observe(el);
      destroy.onDestroy(() => ro.disconnect());
    });
    destroy.onDestroy(() => clearTimeout(this.searchTimer));
  }

  protected rowAt(i: number): HotspotRow | null {
    return this.store.row(i);
  }

  protected groupLabel(g: ProfileGroupBy): string {
    return GROUP_LABELS[g];
  }

  protected measurementText(c: HotspotColumn): string {
    return c.measurement === 'sampled' ? 'sampled (≈): a statistical estimate' : 'exact (=): every occurrence recorded';
  }

  protected ariaSort(c: HotspotColumn): 'ascending' | 'descending' | null {
    const cur = this.store.sort() ?? { metric: this.metric(), of: 'self' as const, descending: true };
    if (!c.sort || !sameSort(cur, c.sort)) return null;
    return (cur.descending ?? cur.metric !== null) ? 'descending' : 'ascending';
  }

  protected sortArrow(c: HotspotColumn): string {
    const s = this.ariaSort(c);
    return s === 'descending' ? ' ▼' : s === 'ascending' ? ' ▲' : '';
  }

  protected setGroupBy(e: Event): void {
    this.store.setGroupBy((e.target as HTMLSelectElement).value as ProfileGroupBy);
  }

  protected onSearch(e: Event): void {
    const value = (e.target as HTMLInputElement).value;
    clearTimeout(this.searchTimer);
    this.searchTimer = setTimeout(() => this.store.setSearch(value), 200);
  }

  protected onScroll(e: Event): void {
    this.scrollTop.set((e.target as HTMLElement).scrollTop);
  }

  /** Activating a row: functions open the butterfly; threads filter the view to that thread. */
  protected primary(r: HotspotRow, i: number): void {
    this.store.select(i);
    if (r.frame) this.nav.butterfly(r.frame);
    else if (r.thread !== null) this.state.setThreads([r.thread]);
  }

  /** Up/Down move between rows (scrolling the virtual window), Home/End jump. */
  protected onKey(e: KeyboardEvent): void {
    const total = this.store.meta()?.totalRows ?? 0;
    const cur = this.store.selected() ?? 0;
    const next = e.key === 'ArrowDown' ? cur + 1 : e.key === 'ArrowUp' ? cur - 1 : e.key === 'Home' ? 0 : e.key === 'End' ? total - 1 : null;
    if (next === null || !total) return;
    e.preventDefault();
    const i = Math.max(0, Math.min(total - 1, next));
    this.store.select(i);
    this.focusRow(i);
  }

  private focusRow(i: number): void {
    const el = this.viewport()?.nativeElement;
    if (!el) return;
    const top = i * ROW_H;
    if (top < el.scrollTop + ROW_H || top > el.scrollTop + el.clientHeight - 2 * ROW_H) {
      el.scrollTop = Math.max(0, top - (el.clientHeight || FALLBACK_H) / 2);
      this.scrollTop.set(el.scrollTop);
    }
    queueMicrotask(() => setTimeout(() => el.querySelector<HTMLElement>(`button[data-row="${i}"]`)?.focus()));
  }
}
