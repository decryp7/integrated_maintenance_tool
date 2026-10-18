import {
  afterNextRender,
  ChangeDetectionStrategy,
  Component,
  computed,
  DestroyRef,
  effect,
  ElementRef,
  inject,
  signal,
  untracked,
  viewChild,
} from '@angular/core';
import { Level, Query, RecordView } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { formatCount, formatDateTime } from '../core/format';
import { describeError } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { LiveTail } from '../workspace/live-tail';
import { LiveFeed } from '../live/live-feed';
import { NewEventsPill } from '../live/new-events-pill';
import { VirtualList } from '../workspace/virtual-list';
import { ClickOutside } from '../shared/click-outside';
import { LEVEL_CODES, LEVEL_VARS } from '../shared/colors';
import { Icon } from '../shared/icon';
import { SettingsStore } from '../settings/settings.store';
import { RecordActions } from '../context/record-actions';
import { FieldFormatters } from '../extensions/field-formatters';
import { MarkerIcon } from '../shared/marker-icon';
import { HighlightRules } from '../projects/bookmarks/highlight';
import { GridSelection } from './grid-selection';
import { contains, EMPTY_SELECTION, extendTo, indices, RowSelection, shift, single, toggle, withFocus } from './row-selection';
import { makeGeometry, navigate, rowAtTop, rowY, scrollToTop, topForRow, topToScroll, visibleWindow, wheelToPixels, clamp } from './scroll-map';
import { NextSteps } from '../ui/next-steps';

/** Row heights for the two grid densities (px). */
const ROW_HEIGHT = { comfortable: 26, compact: 22 } as const;
const HEADER_H = 28;
const PAGE = 500;
const OVERSCAN = 8;
/** Pages kept in memory; the ones farthest from the viewport are dropped first. */
const MAX_PAGES = 40;
/** At most this many selected lines go to actions on several records (e.g. "Infer pattern from lines…"). */
const MAX_ACTION_RECORDS = 200;

/** Anchor requests survive the grid being destroyed (tab switch); only act on new ones (per store). */
const handledAnchorNonce = new WeakMap<object, number>();

interface VisibleRow {
  index: number;
  y: number;
  row: RecordView | null;
}

@Component({
  selector: 'imt-events-grid',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[style.--row-h.px]': 'rowH()' },
  imports: [Icon, MarkerIcon, ClickOutside, VirtualList, NewEventsPill, NextSteps],
  template: `
    <div class="bar">
      <span class="section-title" i18n="@@grid.title">Events</span>
      <span class="mono total">{{ fmt(total()) }}</span>
      <span class="faint" i18n="@@grid.subtitle">of {{ fmt(store.summary()?.total ?? 0) //i18n(ph="total") }} · sorted by time {{ store.descending() ? '↓' : '↑' //i18n(ph="direction") }}</span>
      @if (store.anchorMissed()) {
        <span class="warn" i18n="@@grid.anchorMissed">That event is hidden by the current filters.</span>
      }
      <span class="spacer"></span>
      <button class="btn ghost" (click)="store.toggleSort()" title="Toggle sort order" i18n-title="@@grid.sortToggle">
        <imt-icon [name]="store.descending() ? 'sortDesc' : 'sortAsc'" [size]="14" /> {{ sortLabel() }}
      </button>
      <div class="anchor" (imtClickOutside)="colsOpen.set(false)">
        <button class="btn ghost" (click)="colsOpen.set(!colsOpen())" [attr.aria-expanded]="colsOpen()" i18n="@@grid.columns"><imt-icon name="columns" [size]="14" /> Columns</button>
        @if (colsOpen()) {
          <div class="popover cols">
            <input class="input" placeholder="Filter fields" i18n-placeholder="@@grid.filterFields.placeholder" aria-label="Filter fields" i18n-aria-label="@@grid.filterFields.aria" [value]="colFilter()" (input)="colFilter.set($any($event.target).value)" />
            <imt-virtual-list [items]="colChoices()" [itemHeight]="24" [maxHeight]="300" label="Columns" i18n-label="@@grid.columns.listLabel">
              <ng-template let-f>
                <label class="col-opt">
                  <input type="checkbox" [checked]="store.extraColumns().includes(f)" (change)="toggleColumn(f)" />
                  <span class="mono ellipsis">{{ f }}</span>
                </label>
              </ng-template>
            </imt-virtual-list>
          </div>
        }
      </div>
    </div>

    <imt-new-events-pill />
    <div
      class="viewport"
      #viewport
      tabindex="0"
      role="grid"
      aria-label="Events" i18n-aria-label="@@grid.aria"
      aria-multiselectable="true"
      [attr.aria-rowcount]="total() + 1"
      [attr.aria-colcount]="colCount()"
      (scroll)="onScroll()"
      (wheel)="onWheel($event)"
      (keydown)="onKey($event)"
    >
      <div class="header" role="row" aria-rowindex="1" [style.grid-template-columns]="columns()">
        <span class="hcell" role="columnheader"><span class="sr-only" i18n="@@grid.header.marker">Marker</span></span>
        <span class="hcell" role="columnheader" [attr.aria-sort]="store.descending() ? 'descending' : 'ascending'">
          <button class="sort" (click)="store.toggleSort()" i18n="@@grid.header.time">Time {{ store.descending() ? '↓' : '↑' //i18n(ph="direction") }}</button>
        </span>
        <span class="hcell" role="columnheader" i18n="@@grid.header.level">Level</span>
        <span class="hcell" role="columnheader" i18n="@@grid.header.instance">Instance</span>
        @for (c of store.extraColumns(); track c) {
          <span class="hcell extra" role="columnheader"><span class="ellipsis mono">{{ c }}</span><button class="x" [attr.aria-label]="removeColumnLabel(c)" (click)="store.removeColumn(c)" [attr.title]="removeColumnLabel(c)">×</button></span>
        }
        <span class="hcell" role="columnheader" i18n="@@grid.header.message">Message</span>
      </div>
      <div class="layer" role="presentation" [style.height.px]="viewportH()">
        @for (v of visible(); track v.index) {
          @if (v.row; as r) {
            <div
              class="row"
              role="row"
              [attr.aria-rowindex]="v.index + 2"
              [attr.aria-selected]="isSelected(v.index, r)"
              [class.sel]="isSelected(v.index, r)"
              [class.hl]="!!highlight.colorFor(r)"
              [style.--hl]="highlight.colorFor(r)"
              [style.top.px]="v.y"
              [style.grid-template-columns]="columns()"
              (mousedown)="onRowMouseDown($event)"
              (click)="onRowClick($event, v.index, r)"
              (contextmenu)="openRowMenu($event, v.index, r)"
            >
              <span class="mk" role="gridcell">
                @if (r.markers[0]; as mk) { <imt-marker-icon [kind]="mk" [color]="markerColor(mk)" [label]="markerLabel(mk)" /> }
              </span>
              <span class="mono time" role="gridcell">{{ time(r.ts) }}</span>
              <span class="lvl" role="gridcell"><span class="dot" [style.background]="levelVar(r.level)"></span><span class="mono">{{ code(r.level) }}</span></span>
              <span class="mono inst ellipsis" role="gridcell" [title]="r.group">{{ r.group }}</span>
              @for (c of store.extraColumns(); track c) {
                <span class="mono extra ellipsis" role="gridcell" [title]="cell(r, c)">{{ cell(r, c) }}</span>
              }
              <span class="msg ellipsis" role="gridcell">{{ r.message }}</span>
            </div>
          } @else {
            <div class="row placeholder" role="row" [attr.aria-rowindex]="v.index + 2" [style.top.px]="v.y"><span role="gridcell" class="skeleton" aria-label="Loading" i18n-aria-label="@@grid.loading"></span></div>
          }
        }
        @if (loaded() && total() === 0 && !pageError()) {
          <div class="none faint" role="row" aria-rowindex="2"><span role="gridcell">{{ emptyMessage() }}<imt-next-steps /></span></div>
        }
      </div>
      <div aria-hidden="true" [style.height.px]="spacerH()"></div>
    </div>
    @if (pageError(); as err) {
      <div class="err-card" role="alert">
        <strong i18n="@@grid.error.title">Could not load events</strong>
        <span class="mono">{{ err }}</span>
        <button class="btn" (click)="retry()" i18n="@@grid.error.retry">Retry</button>
      </div>
    }
  `,
  styles: `
    :host { flex: 1; min-width: 0; display: flex; flex-direction: column; background: var(--bg-1); min-height: 0; position: relative; }
    .bar { height: 32px; flex: none; display: flex; align-items: center; gap: 10px; padding: 0 8px 0 14px; border-bottom: 1px solid var(--border); font-size: 12px; }
    .total { color: var(--text-2); }
    .warn { color: var(--warn-text); }
    .spacer { flex: 1; }
    .anchor { position: relative; }
    .cols { right: 0; top: 30px; width: 280px; display: flex; flex-direction: column; gap: 6px; }
    .err-card { position: absolute; top: 90px; left: 50%; transform: translateX(-50%); z-index: 3; display: flex; flex-direction: column; align-items: center; gap: 8px; max-width: 80%; padding: 14px 18px; border: 1px solid var(--border-3); border-radius: 8px; background: var(--bg-3); color: var(--danger-text); text-align: center; }
    .col-opt { width: 100%; display: flex; align-items: center; gap: 8px; height: 24px; color: var(--text-2); font-size: 12px; }
    .header { position: sticky; top: 0; z-index: 2; height: ${HEADER_H}px; display: grid; align-items: center; column-gap: 10px; padding: 0 14px; font-size: 11px; color: var(--muted); border-bottom: 1px solid var(--border); background: var(--bg-2); }
    .hcell { display: flex; align-items: center; gap: 4px; min-width: 0; }
    .sort { border: 0; background: transparent; color: inherit; font-size: 11px; padding: 0; text-align: left; cursor: pointer; }
    .sort:hover { color: var(--text); }
    .x { border: 0; background: transparent; color: var(--faint); cursor: pointer; padding: 0 2px; }
    .viewport { flex: 1; min-height: 0; overflow-y: auto; overflow-x: hidden; position: relative; outline: none; }
    .viewport:focus-visible { box-shadow: inset 0 0 0 1px var(--accent); }
    .layer { position: sticky; top: ${HEADER_H}px; overflow: hidden; }
    .row { position: absolute; left: 0; right: 0; height: var(--row-h, 26px); display: grid; align-items: center; column-gap: 10px; padding: 0 14px; border-bottom: 1px solid var(--grid-line); cursor: default; }
    .row:hover { background: var(--bg-hover); }
    .row.sel { background-color: var(--bg-sel); box-shadow: inset 2px 0 0 var(--accent); }
    /*
     * Highlight rules: a solid colour bar on the left (readable without relying on the tint) plus a faint tint
     * layered over the hover/selection background, so text contrast and selection styling are kept on both themes.
     */
    .row.hl { background-image: linear-gradient(color-mix(in srgb, var(--hl) var(--row-hl-tint, 12%), transparent) 0 0); }
    .row.hl::before { content: ''; position: absolute; left: 0; top: 0; bottom: 0; width: 4px; background: var(--hl); }
    .row.hl.sel { box-shadow: inset 6px 0 0 var(--accent); }
    .mk { display: flex; justify-content: center; }
    .time { font-size: 12px; color: var(--muted); white-space: nowrap; }
    .lvl { display: flex; align-items: center; gap: 6px; font-size: 11px; color: var(--text-2); }
    .inst, .extra { font-size: 12px; color: var(--text-2); }
    .extra { color: var(--muted); }
    .msg { color: var(--text); }
    .placeholder { display: flex; }
    .skeleton { width: 60%; height: 8px; border-radius: 4px; background: var(--bg-hover); }
    .none { position: absolute; top: 24px; left: 0; right: 0; text-align: center; }
  `,
})
export class EventsGrid {
  protected readonly store = inject(WorkspaceStore);
  private readonly backend = inject(ImtBackend);
  private readonly formatters = inject(FieldFormatters);
  private readonly actions = inject(RecordActions);
  private readonly settings = inject(SettingsStore);
  private readonly live = inject(LiveTail);
  /** Live refreshes: whether to reload the rows in view (following) or keep them ("N new events"). */
  private readonly feed = inject(LiveFeed);
  protected readonly highlight = inject(HighlightRules);
  /** Where the grid publishes its multi-selection for palette commands and other panels. */
  private readonly published = inject(GridSelection);
  protected readonly rowH = computed(() => ROW_HEIGHT[this.settings.density()]);
  private readonly viewport = viewChild<ElementRef<HTMLElement>>('viewport');

  protected readonly fmt = formatCount;
  protected readonly colsOpen = signal(false);
  protected readonly colFilter = signal('');
  protected readonly total = signal(0);
  protected readonly loaded = signal(false);
  protected readonly pageError = signal<string | null>(null);
  protected readonly sortLabel = computed(() => (this.store.descending() ? $localize`:@@grid.sort.newest:Newest first` : $localize`:@@grid.sort.oldest:Oldest first`));
  protected readonly emptyMessage = computed(() =>
    this.store.summary()?.sources?.length ? $localize`:@@grid.empty.filtered:No events match the current filters.` : $localize`:@@grid.empty.noFiles:No events: nothing is open yet.`,
  );

  protected removeColumnLabel(column: string): string {
    return $localize`:@@grid.removeColumn:Remove column ${column}:column:`;
  }
  protected readonly viewportH = signal(400);
  /**
   * Offset (px) into the true, uncapped content at the top of the rows area. This, not the element's scrollTop,
   * is the source of truth: with tens of millions of rows one scrollbar pixel covers many rows (see scroll-map.ts).
   */
  protected readonly virtualTop = signal(0);
  private readonly cacheTick = signal(0);

  private pages = new Map<number, RecordView[]>();
  private inflight = new Set<number>();
  private stale = new Set<number>();
  private failed = new Set<number>();
  private generation = 0;
  private query: Query | null = null;
  private queryKey = '';
  private descending = false;
  /** Selected rows by index (multi-select); its focus is the row the inspector shows (`store.selected`). */
  protected readonly selection = signal<RowSelection>(EMPTY_SELECTION);
  /** Row to select once its page arrives (keyboard navigation to a page that is not loaded yet). */
  private pendingSelect: number | null = null;
  /** The scrollTop we last wrote ourselves; scroll events at that position carry no new information. */
  private writtenScroll = Number.NaN;
  /** What to do with the scroll position when the first page of a live refresh arrives. */
  private pendingLive: { oldTotal: number; mode: 'end' | 'shift' | 'keep' } | null = null;

  protected readonly columns = computed(() => {
    const extra = this.store.extraColumns().map(() => '150px').join(' ');
    return `16px 150px 52px 240px ${extra} minmax(0, 1fr)`;
  });

  protected readonly colCount = computed(() => 5 + this.store.extraColumns().length);

  protected readonly colChoices = computed(() => {
    const needle = this.colFilter().toLowerCase();
    return ['@source', '@line', '@template', ...this.store.fieldNames()].filter((f) => f.toLowerCase().includes(needle));
  });

  protected readonly geometry = computed(() => makeGeometry(this.total(), this.rowH(), this.viewportH()));
  protected readonly spacerH = computed(() => this.geometry().maxScroll);

  protected readonly visible = computed<VisibleRow[]>(() => {
    this.cacheTick();
    const g = this.geometry();
    const top = this.virtualTop();
    const { first, last } = visibleWindow(g, top, OVERSCAN);
    const rows: VisibleRow[] = [];
    for (let i = first; i < last; i++) rows.push({ index: i, y: rowY(g, i, top), row: this.rowAt(i) });
    return rows;
  });

  constructor() {
    const destroyRef = inject(DestroyRef);

    // New query, sort order or data: start from the top, except that a live refresh keeps the position.
    effect(() => {
      const query = this.store.effectiveQuery();
      const descending = this.store.descending();
      this.store.dataVersion();
      untracked(() => {
        const key = JSON.stringify(query);
        const sameView = this.queryKey === key && descending === this.descending;
        this.queryKey = key;
        if (sameView && this.live.isLiveRefresh()) {
          if (!this.feed.holdGrid()) this.liveRefresh();
        } else this.reset(query, descending);
      });
    });

    // "N new events" clicked, or following resumed: reload and bring the newest events into view.
    effect(() => {
      const nonce = this.feed.revealNonce();
      untracked(() => {
        if (nonce > 0 && this.query) this.liveRefresh(this.descending ? 'start' : 'end');
      });
    });

    // Fill in pages the viewport needs.
    effect(() => {
      const rows = this.visible();
      untracked(() => {
        for (const v of rows) if (!v.row) this.ensurePage(Math.floor(v.index / PAGE));
      });
    });

    effect(() => {
      const anchor = this.store.anchor();
      if (!anchor || anchor.nonce === handledAnchorNonce.get(this.store)) return;
      handledAnchorNonce.set(this.store, anchor.nonce);
      untracked(() => void this.jumpTo(anchor.id));
    });

    // Something else selected a record (timeline, bookmark, data reset): drop a multi-selection that no longer matches.
    effect(() => {
      const selected = this.store.selected();
      untracked(() => {
        const sel = this.selection();
        if (!selected) {
          if (sel !== EMPTY_SELECTION) this.selection.set(EMPTY_SELECTION);
          return;
        }
        const focusRow = this.rowAt(sel.focus);
        if (focusRow && focusRow.id !== selected.id) this.selection.set(EMPTY_SELECTION);
      });
    });

    // Publish the selected records (and again as their pages load) for commands outside the grid.
    effect(() => {
      this.selection();
      this.cacheTick();
      untracked(() => this.published.publish(this.selectedRecords()));
    });
    destroyRef.onDestroy(() => this.published.clear());

    afterNextRender(() => {
      const el = this.viewport()!.nativeElement;
      const measure = () => this.viewportH.set(Math.max(0, el.clientHeight - HEADER_H));
      const ro = new ResizeObserver(measure);
      ro.observe(el);
      destroyRef.onDestroy(() => ro.disconnect());
      measure();
    });
  }

  private rowAt(i: number): RecordView | null {
    const page = this.pages.get(Math.floor(i / PAGE));
    return page?.[i % PAGE] ?? null;
  }

  private reset(query: Query, descending: boolean): void {
    this.generation++;
    this.query = query;
    this.descending = descending;
    this.pages = new Map();
    this.inflight.clear();
    this.stale.clear();
    this.failed.clear();
    this.pendingLive = null;
    this.pendingSelect = null;
    this.selection.set(EMPTY_SELECTION);
    this.loaded.set(false);
    this.pageError.set(null);
    this.setTop(0);
    this.cacheTick.update((t) => t + 1);
    void this.fetchPage(0, descending);
  }

  /** Live tail changed the data: refetch what is on screen (old rows stay until the new ones arrive) and keep following. */
  private liveRefresh(reveal?: 'start' | 'end'): void {
    if (reveal === 'start') this.setTop(0);
    const g = this.geometry();
    const top = this.virtualTop();
    const oldTotal = this.total();
    const atStart = top < g.rowH;
    const atEnd = top >= g.maxTop - g.rowH;
    const mode = reveal === 'end' ? 'end' : reveal === 'start' ? 'keep' : this.descending ? (atStart ? 'keep' : 'shift') : atEnd ? 'end' : 'keep';
    this.generation++;
    this.inflight.clear();
    this.failed.clear();
    this.pageError.set(null);
    if (mode === 'shift') {
      // New events were inserted above the rows in view: drop the cache so shifted indexes never show wrong rows.
      this.pages = new Map();
      this.stale.clear();
    } else {
      this.stale = new Set(this.pages.keys());
    }
    this.pendingLive = { oldTotal, mode };
    this.cacheTick.update((t) => t + 1);
    const { first, last } = visibleWindow(g, top, 0);
    const wanted = new Set<number>([0, Math.floor(first / PAGE), Math.floor(Math.max(first, last - 1) / PAGE)]);
    if (mode === 'end') wanted.add(Math.floor(Math.max(0, oldTotal - 1) / PAGE));
    for (const page of wanted) this.ensurePage(page);
  }

  private ensurePage(page: number): void {
    if (this.inflight.has(page) || (this.pages.has(page) && !this.stale.has(page))) return;
    void this.fetchPage(page, this.descending);
  }

  private async fetchPage(page: number, descending: boolean): Promise<void> {
    const gen = this.generation;
    const query = this.query;
    if (!query) return;
    this.inflight.add(page);
    try {
      const res = await this.backend.queryPage({ query, offset: page * PAGE, limit: PAGE, descending, anchor: null });
      if (gen !== this.generation) return;
      this.pages.set(page, res.rows);
      this.stale.delete(page);
      this.failed.delete(page);
      this.evictFarPages();
      this.total.set(res.total);
      this.loaded.set(true);
      this.pageError.set(null);
      this.store.setMatchTotal(res.total);
      this.applyLive(res.total);
      this.cacheTick.update((t) => t + 1);
      if (this.pendingSelect !== null) this.selectPending();
    } catch (e) {
      if (gen === this.generation) {
        this.failed.add(page);
        this.pageError.set(describeError(e));
      }
    } finally {
      this.inflight.delete(page);
    }
  }

  /** After the first response of a live refresh: follow the end, or keep the same events in view. */
  private applyLive(newTotal: number): void {
    const live = this.pendingLive;
    if (!live) return;
    this.pendingLive = null;
    const delta = newTotal - live.oldTotal;
    if (live.mode === 'end') {
      this.setTop(Number.POSITIVE_INFINITY, newTotal);
      return;
    }
    if (delta <= 0) return;
    if (live.mode === 'shift') {
      this.setTop(this.virtualTop() + delta * this.rowH(), newTotal);
      this.selection.update((sel) => shift(sel, delta));
    }
  }

  /** Keeps memory bounded while scrolling through millions of rows: drops pages far from the viewport. */
  private evictFarPages(): void {
    if (this.pages.size <= MAX_PAGES) return;
    const centre = Math.floor(rowAtTop(this.geometry(), this.virtualTop()) / PAGE);
    const far = [...this.pages.keys()].filter((p) => !this.inflight.has(p)).sort((a, b) => Math.abs(b - centre) - Math.abs(a - centre));
    for (const page of far.slice(0, this.pages.size - MAX_PAGES)) {
      this.pages.delete(page);
      this.stale.delete(page);
    }
  }

  protected retry(): void {
    this.pageError.set(null);
    if (!this.loaded() || !this.pages.size) {
      this.failed.clear();
      this.generation++;
      this.inflight.clear();
      void this.fetchPage(0, this.descending);
      return;
    }
    const pages = new Set(this.failed);
    this.failed.clear();
    for (const v of this.visible()) if (!v.row) pages.add(Math.floor(v.index / PAGE));
    for (const page of pages) this.ensurePage(page);
  }

  private async jumpTo(id: number): Promise<void> {
    const query = this.query ?? this.store.effectiveQuery();
    const gen = this.generation;
    let res;
    try {
      res = await this.backend.queryPage({ query, offset: 0, limit: 1, descending: this.store.descending(), anchor: id });
    } catch (e) {
      if (gen === this.generation) this.pageError.set(describeError(e));
      return;
    }
    if (gen !== this.generation) return;
    if (res.anchorIndex === null || !res.rows.length) {
      this.store.anchorResolved(null);
      return;
    }
    this.total.set(res.total);
    this.selection.set(single(res.anchorIndex));
    this.store.anchorResolved(res.rows[0] ?? null);
    this.revealRow(res.anchorIndex, 'center', res.total);
  }

  private revealRow(index: number, mode: 'nearest' | 'center', total = this.total()): void {
    const g = makeGeometry(total, this.rowH(), this.viewportH());
    this.setTop(topForRow(g, index, mode, this.virtualTop()), total);
  }

  /** Moves the rows area to a virtual offset and writes the matching scrollbar position. */
  private setTop(top: number, total = this.total()): void {
    const g = makeGeometry(total, this.rowH(), this.viewportH());
    const t = clamp(top, 0, g.maxTop);
    this.virtualTop.set(t);
    const el = this.viewport()?.nativeElement;
    if (!el) return;
    const physical = Math.round(topToScroll(g, t));
    el.scrollTop = physical;
    this.writtenScroll = physical;
  }

  protected onScroll(): void {
    const el = this.viewport()?.nativeElement;
    if (!el) return;
    const g = this.geometry();
    const s = el.scrollTop;
    // Our own writes round to whole pixels; reading them back would lose the row-exact position.
    if (g.scaled && Math.abs(s - this.writtenScroll) <= 1) return;
    this.writtenScroll = Number.NaN;
    this.virtualTop.set(scrollToTop(g, s));
  }

  /** With a scaled scrollbar the wheel moves by pixels of the true content, not of the compressed spacer. */
  protected onWheel(e: WheelEvent): void {
    const g = this.geometry();
    if (!g.scaled || e.ctrlKey || e.deltaY === 0) return;
    e.preventDefault();
    this.setTop(this.virtualTop() + wheelToPixels(e.deltaY, e.deltaMode, g));
  }

  /**
   * A row is selected when it is in the grid's selection; while the grid has none (the record was selected
   * elsewhere, e.g. on the timeline), the record the inspector shows is.
   */
  protected isSelected(index: number, row: RecordView): boolean {
    const sel = this.selection();
    return sel.anchor >= 0 ? contains(sel, index) : row.id === this.store.selected()?.id;
  }

  /** Keeps shift/ctrl-click from also selecting text across rows. */
  protected onRowMouseDown(e: MouseEvent): void {
    if (e.shiftKey || e.ctrlKey || e.metaKey) e.preventDefault();
  }

  /** Click: only this row; Ctrl/Cmd+click: add or remove it; Shift+click: the range from the anchor. */
  protected onRowClick(e: MouseEvent, index: number, row: RecordView): void {
    const sel = this.selection();
    if (e.shiftKey) this.selection.set(extendTo(sel, index));
    else if (e.ctrlKey || e.metaKey) this.selection.set(toggle(sel, index));
    else this.selection.set(single(index));
    this.store.select(row);
  }

  /** Right-click inside the selection acts on all selected rows; elsewhere it selects and acts on that row only. */
  protected openRowMenu(e: MouseEvent, index: number, row: RecordView): void {
    e.preventDefault();
    const sel = this.selection();
    this.selection.set(contains(sel, index) ? withFocus(sel, index) : single(index));
    this.store.select(row);
    this.actions.openFor(e, row, { records: this.selectedRecords() });
  }

  /** The loaded records of the selection in grid order (rows of pages not in memory are skipped), capped. */
  private selectedRecords(): RecordView[] {
    return indices(this.selection(), MAX_ACTION_RECORDS)
      .map((i) => this.rowAt(i))
      .filter((r): r is RecordView => r !== null);
  }

  protected onKey(e: KeyboardEvent): void {
    const total = this.total();
    if (!total) return;
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      const selected = this.store.selected();
      if (selected) {
        e.preventDefault();
        const focus = this.selection().focus;
        const rowEl =
          this.viewport()?.nativeElement.querySelector<HTMLElement>(`.row[aria-rowindex="${focus + 2}"]`) ??
          this.viewport()?.nativeElement.querySelector<HTMLElement>('.row.sel') ??
          (e.currentTarget as HTMLElement);
        const records = contains(this.selection(), focus) ? this.selectedRecords() : [];
        this.actions.openAtElement(rowEl, selected, { records }, e.currentTarget as HTMLElement);
      }
      return;
    }
    const sel = this.selection();
    const next = navigate(this.geometry(), e.key, sel.focus);
    if (next === null) return;
    e.preventDefault();
    // Shift+arrow (or Shift+Page/Home/End) extends the selection from its anchor.
    this.selection.set(e.shiftKey && sel.anchor >= 0 ? extendTo(sel, next) : single(next));
    this.revealRow(next, 'nearest');
    const row = this.rowAt(next);
    if (row) {
      this.pendingSelect = null;
      this.store.select(row);
    } else {
      this.pendingSelect = next;
      this.ensurePage(Math.floor(next / PAGE));
    }
  }

  private selectPending(): void {
    const index = this.pendingSelect;
    if (index === null) return;
    const row = this.rowAt(index);
    if (!row) return;
    this.pendingSelect = null;
    if (this.selection().focus === index) this.store.select(row);
  }

  protected toggleColumn(f: string): void {
    if (this.store.extraColumns().includes(f)) this.store.removeColumn(f);
    else this.store.addColumn(f);
  }

  protected time(ts: number | null): string {
    return formatDateTime(ts, this.store.timezone(), true, this.settings.dateFormat());
  }
  protected levelVar(l: Level): string {
    return LEVEL_VARS[l];
  }
  protected code(l: Level): string {
    return LEVEL_CODES[l];
  }
  protected markerLabel(kind: string): string {
    return this.store.markerKindMap().get(kind)?.label ?? kind;
  }
  protected markerColor(kind: string): string | null {
    return this.store.markerKindMap().get(kind)?.color ?? null;
  }
  protected cell(r: RecordView, c: string): string {
    switch (c) {
      case '@source':
        return r.sourceName;
      case '@line':
        return String(r.line);
      case '@template':
        return r.template ?? '';
      default:
        return r.fields[c] === undefined ? '' : this.formatters.format(c, r.fields[c], r);
    }
  }
}
