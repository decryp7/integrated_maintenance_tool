import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, signal, untracked, viewChild } from '@angular/core';
import { HotspotRow, ProfileGroupBy } from '../../core/api.types';
import { formatCount } from '../../core/format';
import { plural } from '../../i18n/locale';
import { AnalysisState } from '../../analysis/analysis-state';
import { ProfileViewState } from '../profile-view-state.store';
import { ActiveProfile, ProfilingNavigator } from '../profiling-navigator';
import { CaptureNotes, MeasurementLegend, MeasurementTag, NoCapture, ProfileBar } from '../profiling-shared';
import { PROFILING_STYLES } from '../profiling-styles';
import { GROUP_LABELS, GROUP_NOUNS, HotspotColumn, hotspotColumns, sameSort } from './hotspot-columns';
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
  selector: 'debrief-hotspots-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, ProfileBar, MeasurementTag, MeasurementLegend, CaptureNotes, NoCapture],
  providers: [HotspotsStore],
  template: `
    @if (active.profile(); as p) {
      <debrief-profile-bar />
      <debrief-capture-notes [notes]="p.notes" [name]="p.name" />
      <div class="bar">
        <span class="section-title" i18n="@@profiling.hotspots.title">Hotspots</span>
        <label class="inline"><ng-container i18n="@@profiling.hotspots.groupBy">Group by</ng-container>&ngsp;
          <select aria-label="Group by" i18n-aria-label="@@profiling.hotspots.groupByLabel" (change)="setGroupBy($event)">
            @for (g of groups; track g) { <option [value]="g" [selected]="g === store.groupBy()">{{ groupLabel(g) }}</option> }
          </select>
        </label>
        <input type="search" placeholder="Search…" i18n-placeholder="@@profiling.hotspots.searchPlaceholder" aria-label="Search hotspots" i18n-aria-label="@@profiling.hotspots.searchLabel" [value]="store.search()" (input)="onSearch($event)" />
        @if (store.meta(); as m) {
          <span class="muted" data-testid="hotspots-count">{{ countText(m.totalRows, m.samplesInView) }}</span>
        }
        <span class="spacer"></span>
        <debrief-measurement-legend />
      </div>
      <debrief-analysis-state [loading]="store.loading()" [error]="store.error()" [hasData]="!!store.meta()" [empty]="store.meta()?.totalRows === 0" emptyText="No samples match this view (range, threads or search)." i18n-emptyText="@@profiling.hotspots.empty" (retry)="store.retry()">
        <div class="viewport" #viewport tabindex="0" role="region" aria-label="Hotspot rows" i18n-aria-label="@@profiling.hotspots.rowsRegion" (scroll)="onScroll($event)">
          <table class="grid" [attr.aria-rowcount]="(store.meta()?.totalRows ?? 0) + 1" aria-label="Hotspots" i18n-aria-label="@@profiling.hotspots.tableLabel">
            <thead>
              <tr aria-rowindex="1">
                @for (c of columns(); track c.id) {
                  <th scope="col" [class.num]="c.numeric" [attr.aria-sort]="ariaSort(c)" [title]="headerTitle(c)">
                    @if (c.sort) {
                      <button type="button" (click)="store.sortBy(c.sort)">{{ c.label }}<span aria-hidden="true">{{ sortArrow(c) }}</span></button>
                    } @else {
                      <span>{{ c.label }}</span>
                    }
                    @if (c.measurement; as m) { <debrief-measurement [m]="m" /> }
                  </th>
                }
                <th scope="col"><span class="sr" i18n="@@profiling.hotspots.actions">Actions</span></th>
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
                          <button class="mini" type="button" [attr.aria-label]="treeLabel(r.name)" (click)="nav.showInTree(r, metric())" i18n="@@profiling.hotspots.tree">Tree</button>
                          <button class="mini" type="button" [attr.aria-label]="butterflyLabel(r.name)" (click)="nav.butterfly(r.frame)" i18n="@@profiling.hotspots.butterfly">Butterfly</button>
                          <button class="mini" type="button" [attr.aria-label]="flameLabel(r.name)" (click)="nav.showInFlame(r.frame)" i18n="@@profiling.hotspots.flame">Flame</button>
                          <button class="mini" type="button" [attr.aria-label]="focusLabel(r.name)" (click)="nav.focusInFlame(r.frame, metric())" i18n="@@profiling.hotspots.focus">Focus</button>
                        </span>
                      }
                    </td>
                  } @else {
                    <td class="muted" [attr.colspan]="columns().length + 1" i18n="@@profiling.hotspots.loading">Loading…</td>
                  }
                </tr>
              }
              @if (win().after) { <tr class="spacer" aria-hidden="true" [style.height.px]="win().after"><td [attr.colspan]="columns().length + 1"></td></tr> }
            </tbody>
          </table>
        </div>
      </debrief-analysis-state>
    } @else {
      <debrief-no-capture text="Open a performance capture to see its hotspots." i18n-text="@@profiling.hotspots.noCapture" />
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
    return c.measurement === 'sampled'
      ? $localize`:@@profiling.hotspots.measurement.sampled:sampled (≈): a statistical estimate`
      : $localize`:@@profiling.hotspots.measurement.exact:exact (=): every occurrence recorded`;
  }

  protected headerTitle(c: HotspotColumn): string {
    if (!c.measurement) return c.help;
    const help = c.help;
    const measurement = this.measurementText(c);
    return $localize`:@@profiling.hotspots.headerTitle:${help}:help: — ${measurement}:measurement:`;
  }

  protected countText(rows: number, samples: number): string {
    const nouns = GROUP_NOUNS[this.store.groupBy()];
    const count = this.count(rows);
    const noun = plural(rows, { one: nouns.one, other: nouns.other });
    const inView = this.count(samples);
    return $localize`:@@profiling.hotspots.count:${count}:count: ${noun}:noun: · ${inView}:samples: samples in view`;
  }

  protected treeLabel(name: string): string {
    return $localize`:@@profiling.hotspots.treeLabel:Show ${name}:name: in the call tree`;
  }

  protected butterflyLabel(name: string): string {
    return $localize`:@@profiling.hotspots.butterflyLabel:Callers and callees of ${name}:name:`;
  }

  protected flameLabel(name: string): string {
    return $localize`:@@profiling.hotspots.flameLabel:Show ${name}:name: in the flame graph`;
  }

  protected focusLabel(name: string): string {
    return $localize`:@@profiling.hotspots.focusLabel:Zoom the flame graph to ${name}:name:`;
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
