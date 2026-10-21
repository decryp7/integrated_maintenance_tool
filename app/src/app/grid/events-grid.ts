import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, effect, ElementRef, inject, signal, untracked, viewChild } from '@angular/core';
import { RecordView } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { formatCount, formatDateTime } from '../core/format';
import { describeError } from '../core/logger';
import { QueryStore } from '../core/workspace/query.store';
import { SelectionStore } from '../core/workspace/selection.store';
import { UiPrefsStore } from '../core/workspace/ui-prefs.store';
import { WorkspaceDataStore } from '../core/workspace/workspace-data.store';
import { LiveTail } from '../workspace/live-tail';
import { LiveFeed } from '../live/live-feed';
import { NewEventsPill } from '../live/new-events-pill';
import { VirtualList } from '../workspace/virtual-list';
import { ClickOutside } from '../shared/click-outside';
import { LEVEL_CODES, LEVEL_VARS } from '../shared/colors';
import { Icon } from '../shared/icon';
import { SettingsStore } from '../settings/settings.store';
import { ThemeService } from '../core/workspace/theme.service';
import { px } from '../theme/theme-tokens';
import { RecordActions } from '../context/record-actions';
import { FieldFormatters } from '../extensions/field-formatters';
import { MarkerIcon } from '../shared/marker-icon';
import { HighlightRules } from '../projects/bookmarks/highlight';
import { GridSelection } from './grid-selection';
import { contains, EMPTY_SELECTION, extendTo, indices, RowSelection, shift, single, toggle, withFocus } from './row-selection';
import { makeGeometry, navigate, rowAtTop, rowY, scrollToTop, topForRow, topToScroll, visibleWindow, wheelToPixels, clamp } from './scroll-map';
import { NextSteps } from '../ui/next-steps';
import { SourceColors, SourceMark } from '../sources/source-colors.store';
import { stripeBackground } from '../sources/source-colors';
import { LiveArrival, liveMode, PagedRowsStore } from './paged-rows.store';

export { SCROLL_IDLE_MS } from './paged-rows.store';

/** Row heights for the two grid densities (px). */
/** Row height when the theme gives none (the theme's `--row-h` per density wins). */
const FALLBACK_ROW_H = 24;
const HEADER_H = 28;
const OVERSCAN = 8;
/** At most this many selected lines go to actions on several records (e.g. "Infer pattern from lines…"). */
const MAX_ACTION_RECORDS = 200;

let gridSeq = 0;

/** Anchor requests survive the grid being destroyed (tab switch); only act on new ones (per store). */
const handledAnchorNonce = new WeakMap<object, number>();

/** What one rendered row shows: everything the template needs, computed once per change instead of per binding. */
export interface RowVm {
  kind: 'row';
  index: number;
  y: number;
  row: RecordView;
  /** Element id (the grid names the focused row with aria-activedescendant). */
  id: string;
  selected: boolean;
  /** Colour of the first highlight rule that matches, if any. */
  highlight: string | null;
  /** Source / instance colour stripe. */
  mark: SourceMark | null;
  stripe: string | null;
  marker: { kind: string; color: string | null; label: string } | null;
  time: string;
  /** The event has no timestamp: it is in the grid but not on the timeline (the time cell says so). */
  untimed: boolean;
  levelVar: string;
  levelCode: string;
  /** The extra columns' texts, in column order. */
  cells: { column: string; text: string }[];
}

/** A row whose page has not arrived. */
export interface PlaceholderVm {
  kind: 'placeholder';
  index: number;
  y: number;
}

export type GridRowVm = RowVm | PlaceholderVm;

@Component({
  selector: 'imt-events-grid',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[style.--row-h.px]': 'rowH()' },
  imports: [Icon, MarkerIcon, ClickOutside, VirtualList, NewEventsPill, NextSteps],
  template: `
    <div class="bar">
      <span class="section-title" i18n="@@grid.title">Events</span>
      <span class="mono total">{{ fmt(total()) }}</span>
      <span class="faint" i18n="@@grid.subtitle">of {{ fmt(data.summary()?.total ?? 0) //i18n(ph="total") }} · sorted by time {{ prefs.descending() ? '↓' : '↑' //i18n(ph="direction") }}</span>
      @if (sel.anchorMissed()) {
        <span class="warn" i18n="@@grid.anchorMissed">That event is hidden by the current filters.</span>
      }
      <span class="spacer"></span>
      <button class="btn ghost" (click)="prefs.toggleSort()" title="Toggle sort order" i18n-title="@@grid.sortToggle">
        <imt-icon [name]="prefs.descending() ? 'sortDesc' : 'sortAsc'" [size]="14" /> {{ sortLabel() }}
      </button>
      <div class="anchor" (imtClickOutside)="colsOpen.set(false)">
        <button class="btn ghost" (click)="colsOpen.set(!colsOpen())" [attr.aria-expanded]="colsOpen()" i18n="@@grid.columns"><imt-icon name="columns" [size]="14" /> Columns</button>
        @if (colsOpen()) {
          <div class="popover cols">
            <input class="input" placeholder="Filter fields" i18n-placeholder="@@grid.filterFields.placeholder" aria-label="Filter fields" i18n-aria-label="@@grid.filterFields.aria" [value]="colFilter()" (input)="colFilter.set($any($event.target).value)" />
            <imt-virtual-list [items]="colChoices()" [itemHeight]="24" [maxHeight]="300" label="Columns" i18n-label="@@grid.columns.listLabel">
              <ng-template let-f>
                <label class="col-opt">
                  <input type="checkbox" [checked]="prefs.extraColumns().includes(f)" (change)="toggleColumn(f)" />
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
      [attr.aria-activedescendant]="activeRowId()"
      (scroll)="onScroll()"
      (wheel)="onWheel($event)"
      (keydown)="onKey($event)"
    >
      <div class="header" role="row" aria-rowindex="1" [style.grid-template-columns]="columns()">
        <span class="hcell" role="columnheader"><span class="sr-only" i18n="@@grid.header.marker">Marker</span></span>
        <span class="hcell" role="columnheader" [attr.aria-sort]="prefs.descending() ? 'descending' : 'ascending'">
          <button class="sort" (click)="prefs.toggleSort()" i18n="@@grid.header.time">Time {{ prefs.descending() ? '↓' : '↑' //i18n(ph="direction") }}</button>
        </span>
        <span class="hcell" role="columnheader" i18n="@@grid.header.level">Level</span>
        <span class="hcell" role="columnheader" i18n="@@grid.header.instance">Instance</span>
        @for (c of prefs.extraColumns(); track c) {
          <span class="hcell extra" role="columnheader"><span class="ellipsis mono">{{ c }}</span><button class="x" [attr.aria-label]="removeColumnLabel(c)" (click)="prefs.removeColumn(c)" [attr.title]="removeColumnLabel(c)">×</button></span>
        }
        <span class="hcell" role="columnheader" i18n="@@grid.header.message">Message</span>
      </div>
      <div class="layer" role="presentation" [style.height.px]="viewportH()">
        @for (v of rowVms(); track v.index) {
          @if (v.kind === 'row') {
            <div
              class="row"
              role="row"
              [id]="v.id"
              [class.focused]="v.id === activeRowId()"
              [attr.aria-rowindex]="v.index + 2"
              [attr.aria-selected]="v.selected"
              [class.sel]="v.selected"
              [class.hl]="!!v.highlight"
              [class.marked]="colors.enabled()"
              [style.--hl]="v.highlight"
              [style.top.px]="v.y"
              [style.grid-template-columns]="columns()"
              (mousedown)="onRowMouseDown($event)"
              (click)="onRowClick($event, v.index, v.row)"
              (contextmenu)="openRowMenu($event, v.index, v.row)"
            >
              <span class="mk" role="gridcell">
                @if (v.mark; as m) {
                  <span class="srcmark" role="img" data-testid="source-stripe" [style.background]="v.stripe" [attr.data-pattern]="m.swatch.pattern" [attr.aria-label]="m.label" [title]="m.label"></span>
                }
                @if (v.marker; as mk) { <imt-marker-icon [kind]="mk.kind" [color]="mk.color" [label]="mk.label" /> }
              </span>
              @if (v.untimed) {
                <span class="mono time untimed" role="gridcell" title="This event has no timestamp, so it is not on the timeline" i18n-title="@@grid.untimedTitle" i18n="@@grid.untimed">no time</span>
              } @else {
                <span class="mono time" role="gridcell">{{ v.time }}</span>
              }
              <span class="lvl" role="gridcell"><span class="dot" [style.background]="v.levelVar"></span><span class="mono">{{ v.levelCode }}</span></span>
              <span class="mono inst ellipsis" role="gridcell" [title]="v.row.group">{{ v.row.group }}</span>
              @for (c of v.cells; track c.column) {
                <span class="mono extra ellipsis" role="gridcell" [title]="c.text">{{ c.text }}</span>
              }
              <span class="msg ellipsis" role="gridcell">@if (v.row.searchMatch === 'partial') {<span class="partial" data-testid="partial-match" title="Partial match: the search text appears inside a word" i18n-title="@@grid.partialMatchTitle" i18n="@@grid.partialMatch">partial</span>}{{ v.row.message }}</span>
            </div>
          } @else {
            <div class="row placeholder" role="row" [attr.aria-rowindex]="v.index + 2" [style.top.px]="v.y"><span role="gridcell" class="skeleton" aria-label="Loading" i18n-aria-label="@@grid.loading"></span></div>
          }
        }
        @if (rows.loaded() && total() === 0 && !rows.error()) {
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
    .bar { height: 32px; flex: none; display: flex; align-items: center; gap: 10px; padding: 0 8px 0 14px; border-bottom: 1px solid var(--border); font-size: var(--fs-sm); }
    .total { color: var(--text-2); }
    .warn { color: var(--warn-text); }
    .spacer { flex: 1; }
    .anchor { position: relative; }
    .cols { right: 0; top: 30px; width: 280px; display: flex; flex-direction: column; gap: 6px; }
    .err-card { position: absolute; top: 90px; left: 50%; transform: translateX(-50%); z-index: 3; display: flex; flex-direction: column; align-items: center; gap: 8px; max-width: 80%; padding: 14px 18px; border: 1px solid var(--border-3); border-radius: 8px; background: var(--bg-3); color: var(--danger-text); text-align: center; }
    .col-opt { width: 100%; display: flex; align-items: center; gap: 8px; height: 24px; color: var(--text-2); font-size: var(--fs-sm); }
    .header { position: sticky; top: 0; z-index: 2; height: ${HEADER_H}px; display: grid; align-items: center; column-gap: 10px; padding: 0 14px; font-size: var(--fs-xs); color: var(--muted); border-bottom: 1px solid var(--border); background: var(--bg-2); }
    .hcell { display: flex; align-items: center; gap: 4px; min-width: 0; }
    .sort { border: 0; background: transparent; color: inherit; font-size: var(--fs-xs); padding: 0; text-align: left; cursor: pointer; }
    .sort:hover { color: var(--text); }
    .x { border: 0; background: transparent; color: var(--faint); cursor: pointer; padding: 0 2px; }
    .viewport { flex: 1; min-height: 0; overflow-y: auto; overflow-x: hidden; position: relative; outline: none; }
    .viewport:focus-visible { box-shadow: inset 0 0 0 1px var(--accent); }
    .layer { position: sticky; top: ${HEADER_H}px; overflow: hidden; }
    .row { position: absolute; left: 0; right: 0; height: var(--row-h, 26px); display: grid; align-items: center; column-gap: 10px; padding: 0 14px; border-bottom: 1px solid var(--grid-line); cursor: default; }
    .row:hover { background: var(--bg-hover); }
    .row.sel { background-color: var(--bg-sel); box-shadow: inset 2px 0 0 var(--accent); }
    /* The row keyboard focus is on (aria-activedescendant), while the grid has focus. */
    .viewport:focus-visible .row.focused { outline: 1px solid var(--accent); outline-offset: -1px; }
    /*
     * Highlight rules: a solid colour bar on the left (readable without relying on the tint) plus a faint tint
     * layered over the hover/selection background, so text contrast and selection styling are kept on both themes.
     */
    .row.hl { background-image: linear-gradient(color-mix(in srgb, var(--hl) var(--row-hl-tint, 12%), transparent) 0 0); }
    .row.hl::before { content: ''; position: absolute; left: 0; top: 0; bottom: 0; width: 4px; background: var(--hl); }
    .row.hl.sel { box-shadow: inset 6px 0 0 var(--accent); }
    /* Source / instance colour: a 4px stripe at the row start (dashed or dotted past four colours), before any highlight bar. */
    .srcmark { position: absolute; left: 0; top: 0; bottom: 0; width: 4px; }
    .srcmark::after { content: ''; position: absolute; inset: 0 -6px 0 0; }
    .row.marked.hl::before { left: 4px; }
    .row.marked.sel { box-shadow: inset 6px 0 0 var(--accent); }
    .row.marked.hl.sel { box-shadow: inset 10px 0 0 var(--accent); }
    .mk { display: flex; justify-content: center; }
    .time { font-size: var(--fs-sm); color: var(--muted); white-space: nowrap; }
    .time.untimed { font-style: italic; color: var(--faint); }
    .partial { font-size: var(--fs-xs); color: var(--faint); border: 1px dashed var(--border-2); border-radius: 4px; padding: 0 4px; margin-right: 6px; }
    .lvl { display: flex; align-items: center; gap: 6px; font-size: var(--fs-xs); color: var(--text-2); }
    .inst, .extra { font-size: var(--fs-sm); color: var(--text-2); }
    .extra { color: var(--muted); }
    .msg { color: var(--text); }
    .placeholder { display: flex; }
    .skeleton { width: 60%; height: 8px; border-radius: 4px; background: var(--bg-hover); }
    .none { position: absolute; top: 24px; left: 0; right: 0; text-align: center; }
  `,
})
export class EventsGrid {
  protected readonly data = inject(WorkspaceDataStore);
  private readonly queryStore = inject(QueryStore);
  protected readonly sel = inject(SelectionStore);
  protected readonly prefs = inject(UiPrefsStore);
  private readonly backend = inject(ImtBackend);
  private readonly formatters = inject(FieldFormatters);
  private readonly actions = inject(RecordActions);
  private readonly settings = inject(SettingsStore);
  private readonly live = inject(LiveTail);
  /** Live refreshes: whether to reload the rows in view (following) or keep them ("N new events"). */
  private readonly feed = inject(LiveFeed);
  private readonly highlight = inject(HighlightRules);
  /** Source / instance colour of each row (View › Source colours). */
  protected readonly colors = inject(SourceColors);
  /** Where the grid publishes its multi-selection for palette commands and other panels. */
  private readonly published = inject(GridSelection);
  private readonly theme = inject(ThemeService);
  /** The theme's row height for the density in effect (the virtual scroll needs it in pixels). */
  protected readonly rowH = computed(() => px(this.theme.tokens()['row-h']) || FALLBACK_ROW_H);
  private readonly viewport = viewChild<ElementRef<HTMLElement>>('viewport');

  /** The rows: paging, eviction, generations and live refreshes (no DOM). */
  protected readonly rows: PagedRowsStore = new PagedRowsStore({
    fetch: (request) => this.backend.queryPage(request),
    onTotal: (total) => this.sel.setMatchTotal(total),
    onLive: (arrival) => this.applyLive(arrival),
    onPage: () => this.selectPending(),
    centre: (): number => rowAtTop(this.geometry(), this.virtualTop()),
  });

  protected readonly fmt = formatCount;
  protected readonly colsOpen = signal(false);
  protected readonly colFilter = signal('');
  protected readonly total = this.rows.total;
  /** Why loading rows failed: a page or a jump to a record. */
  private readonly jumpError = signal<string | null>(null);
  protected readonly pageError = computed(() => this.rows.error() ?? this.jumpError());
  protected readonly sortLabel = computed(() => (this.prefs.descending() ? $localize`:@@grid.sort.newest:Newest first` : $localize`:@@grid.sort.oldest:Oldest first`));
  protected readonly emptyMessage = computed(() =>
    this.data.summary()?.sources?.length ? $localize`:@@grid.empty.filtered:No events match the current filters.` : $localize`:@@grid.empty.noFiles:No events: nothing is open yet.`,
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

  private queryKey = '';
  /** Selected rows by index (multi-select); its focus is the row the inspector shows (`selected`). */
  protected readonly selection = signal<RowSelection>(EMPTY_SELECTION);
  /** Row to select once its page arrives (keyboard navigation to a page that is not loaded yet). */
  private pendingSelect: number | null = null;
  /** The scrollTop we last wrote ourselves; scroll events at that position carry no new information. */
  private writtenScroll = Number.NaN;

  protected readonly columns = computed(() => {
    const extra = this.prefs.extraColumns().map(() => '150px').join(' ');
    return `16px 150px 52px 240px ${extra} minmax(0, 1fr)`;
  });

  protected readonly colCount = computed(() => 5 + this.prefs.extraColumns().length);

  protected readonly colChoices = computed(() => {
    const needle = this.colFilter().toLowerCase();
    return ['@source', '@line', '@template', ...this.data.fieldNames()].filter((f) => f.toLowerCase().includes(needle));
  });

  protected readonly geometry = computed(() => makeGeometry(this.total(), this.rowH(), this.viewportH()));
  protected readonly spacerH = computed(() => this.geometry().maxScroll);

  /** The row indexes in view (with overscan) and their positions, with the rows loaded so far. */
  private readonly visible = computed(() => {
    this.rows.version();
    const g = this.geometry();
    const top = this.virtualTop();
    const { first, last } = visibleWindow(g, top, OVERSCAN);
    const out: { index: number; y: number; row: RecordView | null }[] = [];
    for (let i = first; i < last; i++) out.push({ index: i, y: rowY(g, i, top), row: this.rows.rowAt(i) });
    return out;
  });

  private readonly rowIdPrefix = `events-row-${++gridSeq}-`;

  /**
   * The row keyboard focus is on (the selection's focus), named by the grid with aria-activedescendant so screen
   * readers announce it while focus stays on the grid. Null while that row is not rendered (scrolled away or loading).
   */
  protected readonly activeRowId = computed(() => {
    const focus = this.selection().focus;
    if (focus < 0) return null;
    const vm = this.rowVms().find((v) => v.index === focus);
    return vm?.kind === 'row' ? vm.id : null;
  });

  /** What each row in view shows (review #9: no per-row method calls in the template). */
  protected readonly rowVms = computed<GridRowVm[]>(() => {
    const sel = this.selection();
    const selectedId = this.sel.selected()?.id;
    const tz = this.prefs.timezone();
    const dateFormat = this.settings.dateFormat();
    const columns = this.prefs.extraColumns();
    const kinds = this.data.markerKindMap();
    return this.visible().map(({ index, y, row }): GridRowVm => {
      if (!row) return { kind: 'placeholder', index, y };
      const mark = this.colors.forRecord(row);
      const first = row.markers[0];
      const kind = first === undefined ? undefined : kinds.get(first);
      return {
        kind: 'row',
        index,
        y,
        row,
        id: this.rowIdPrefix + index,
        // In the grid's selection; while the grid has none (selected elsewhere, e.g. on the timeline), the inspector's record.
        selected: sel.anchor >= 0 ? contains(sel, index) : row.id === selectedId,
        highlight: this.highlight.colorFor(row),
        mark,
        stripe: mark ? stripeBackground(mark.swatch) : null,
        marker: first === undefined ? null : { kind: first, color: kind?.color ?? null, label: kind?.label ?? first },
        time: formatDateTime(row.ts, tz, true, dateFormat),
        untimed: row.tsState === 'untimed' || row.ts === null,
        levelVar: LEVEL_VARS[row.level],
        levelCode: LEVEL_CODES[row.level],
        cells: columns.map((column) => ({ column, text: this.cell(row, column) })),
      };
    });
  });

  constructor() {
    const destroyRef = inject(DestroyRef);
    destroyRef.onDestroy(() => this.rows.dispose());

    // New query, sort order or data: start from the top, except that a live refresh keeps the position.
    effect(() => {
      const query = this.queryStore.effectiveQuery();
      const descending = this.prefs.descending();
      this.data.dataVersion();
      untracked(() => {
        const key = JSON.stringify(query);
        const sameView = this.queryKey === key && descending === this.rows.descending;
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
        if (nonce > 0 && this.rows.query) this.liveRefresh(this.rows.descending ? 'start' : 'end');
      });
    });

    // Fill in pages the viewport needs; while the user scrolls, only once scrolling pauses. Other changes (data, a
    // jump) fetch at once.
    effect(() => {
      const missing = this.visible().some((v) => !v.row);
      if (!missing) return;
      untracked(() => this.rows.fill(() => this.missingRows()));
    });

    effect(() => {
      const anchor = this.sel.anchor();
      if (!anchor || anchor.nonce === handledAnchorNonce.get(this.sel)) return;
      handledAnchorNonce.set(this.sel, anchor.nonce);
      untracked(() => void this.jumpTo(anchor.id));
    });

    // Something else selected a record (timeline, bookmark, data reset): drop a multi-selection that no longer matches.
    effect(() => {
      const selected = this.sel.selected();
      untracked(() => {
        const sel = this.selection();
        if (!selected) {
          if (sel !== EMPTY_SELECTION) this.selection.set(EMPTY_SELECTION);
          return;
        }
        const focusRow = this.rows.rowAt(sel.focus);
        if (focusRow && focusRow.id !== selected.id) this.selection.set(EMPTY_SELECTION);
      });
    });

    // Publish the selected records (and again as their pages load) for commands outside the grid.
    effect(() => {
      this.selection();
      this.rows.version();
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

  private missingRows(): number[] {
    return this.visible()
      .filter((v) => !v.row)
      .map((v) => v.index);
  }

  private reset(query: Parameters<PagedRowsStore['reset']>[0], descending: boolean): void {
    this.pendingSelect = null;
    this.jumpError.set(null);
    this.selection.set(EMPTY_SELECTION);
    this.setTop(0);
    this.rows.reset(query, descending);
  }

  /** Live tail changed the data: refetch what is on screen (old rows stay until the new ones arrive) and keep following. */
  private liveRefresh(reveal?: 'start' | 'end'): void {
    if (reveal === 'start') this.setTop(0);
    const g = this.geometry();
    const top = this.virtualTop();
    const mode = liveMode(this.rows.descending, top < g.rowH, top >= g.maxTop - g.rowH, reveal);
    const { first, last } = visibleWindow(g, top, 0);
    this.jumpError.set(null);
    this.rows.liveRefresh(mode, first, last);
  }

  /** After the first response of a live refresh: follow the end, or keep the same events in view. */
  private applyLive({ mode, newTotal, delta }: LiveArrival): void {
    if (mode === 'end') {
      this.setTop(Number.POSITIVE_INFINITY, newTotal);
      return;
    }
    if (delta <= 0) return;
    if (mode === 'shift') {
      this.setTop(this.virtualTop() + delta * this.rowH(), newTotal);
      this.selection.update((sel) => shift(sel, delta));
    }
  }

  protected retry(): void {
    this.jumpError.set(null);
    this.rows.retry(this.missingRows());
  }

  private async jumpTo(id: number): Promise<void> {
    const query = this.rows.query ?? this.queryStore.effectiveQuery();
    const gen = this.rows.generation;
    let res;
    try {
      res = await this.backend.queryPage({ query, offset: 0, limit: 1, descending: this.prefs.descending(), anchor: id });
    } catch (e) {
      if (gen === this.rows.generation) this.jumpError.set(describeError(e));
      return;
    }
    if (gen !== this.rows.generation) return;
    if (res.anchorIndex === null || !res.rows.length) {
      this.sel.anchorResolved(null);
      return;
    }
    this.rows.setTotal(res.total);
    this.selection.set(single(res.anchorIndex));
    this.sel.anchorResolved(res.rows[0] ?? null);
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
    this.rows.noteScroll();
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
    this.sel.select(row);
  }

  /** Right-click inside the selection acts on all selected rows; elsewhere it selects and acts on that row only. */
  protected openRowMenu(e: MouseEvent, index: number, row: RecordView): void {
    e.preventDefault();
    const sel = this.selection();
    this.selection.set(contains(sel, index) ? withFocus(sel, index) : single(index));
    this.sel.select(row);
    this.actions.openFor(e, row, { records: this.selectedRecords() });
  }

  /** The loaded records of the selection in grid order (rows of pages not in memory are skipped), capped. */
  private selectedRecords(): RecordView[] {
    return indices(this.selection(), MAX_ACTION_RECORDS)
      .map((i) => this.rows.rowAt(i))
      .filter((r): r is RecordView => r !== null);
  }

  protected onKey(e: KeyboardEvent): void {
    const total = this.total();
    if (!total) return;
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      const selected = this.sel.selected();
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
    const row = this.rows.rowAt(next);
    if (row) {
      this.pendingSelect = null;
      this.sel.select(row);
    } else {
      this.pendingSelect = next;
      this.rows.ensurePage(this.rows.pageOf(next));
    }
  }

  private selectPending(): void {
    const index = this.pendingSelect;
    if (index === null) return;
    const row = this.rows.rowAt(index);
    if (!row) return;
    this.pendingSelect = null;
    if (this.selection().focus === index) this.sel.select(row);
  }

  protected toggleColumn(f: string): void {
    if (this.prefs.extraColumns().includes(f)) this.prefs.removeColumn(f);
    else this.prefs.addColumn(f);
  }

  private cell(r: RecordView, c: string): string {
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
