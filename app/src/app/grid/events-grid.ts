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
import { WorkspaceStore } from '../core/workspace.store';
import { ClickOutside } from '../shared/click-outside';
import { LEVEL_CODES, LEVEL_VARS } from '../shared/colors';
import { Icon } from '../shared/icon';
import { SettingsStore } from '../settings/settings.store';
import { RecordActions } from '../context/record-actions';
import { FieldFormatters } from '../extensions/field-formatters';
import { MarkerIcon } from '../shared/marker-icon';

/** Row heights for the two grid densities (px). */
const ROW_HEIGHT = { comfortable: 26, compact: 22 } as const;
const HEADER_H = 28;
const PAGE = 500;
/** Browsers cap element heights (~33M px in Chromium); beyond this the scrollbar is scaled. */
const MAX_PX = 15_000_000;
const OVERSCAN = 8;

/** Anchor requests survive the grid being destroyed (tab switch); only act on new ones. */
let handledAnchorNonce = 0;

interface VisibleRow {
  index: number;
  y: number;
  row: RecordView | null;
}

@Component({
  selector: 'imt-events-grid',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { '[style.--row-h.px]': 'rowH()' },
  imports: [Icon, MarkerIcon, ClickOutside],
  template: `
    <div class="bar">
      <span class="section-title">Events</span>
      <span class="mono total">{{ fmt(total()) }}</span>
      <span class="faint">of {{ fmt(store.summary()?.total ?? 0) }} · sorted by time {{ store.descending() ? '↓' : '↑' }}</span>
      @if (store.anchorMissed()) {
        <span class="warn">That event is hidden by the current filters.</span>
      }
      <span class="spacer"></span>
      <button class="btn ghost" (click)="store.toggleSort()" title="Toggle sort order">
        <imt-icon [name]="store.descending() ? 'sortDesc' : 'sortAsc'" [size]="14" /> {{ store.descending() ? 'Newest first' : 'Oldest first' }}
      </button>
      <div class="anchor" (imtClickOutside)="colsOpen.set(false)">
        <button class="btn ghost" (click)="colsOpen.set(!colsOpen())" [attr.aria-expanded]="colsOpen()"><imt-icon name="columns" [size]="14" /> Columns</button>
        @if (colsOpen()) {
          <div class="popover cols">
            <input class="input" placeholder="Filter fields" aria-label="Filter fields" [value]="colFilter()" (input)="colFilter.set($any($event.target).value)" />
            <div class="col-list">
              @for (f of colChoices(); track f) {
                <label class="col-opt">
                  <input type="checkbox" [checked]="store.extraColumns().includes(f)" (change)="toggleColumn(f)" />
                  <span class="mono ellipsis">{{ f }}</span>
                </label>
              }
            </div>
          </div>
        }
      </div>
    </div>

    <div
      class="viewport"
      #viewport
      tabindex="0"
      role="grid"
      aria-label="Events"
      [attr.aria-rowcount]="total() + 1"
      [attr.aria-colcount]="colCount()"
      (scroll)="onScroll()"
      (keydown)="onKey($event)"
    >
      <div class="header" role="row" aria-rowindex="1" [style.grid-template-columns]="columns()">
        <span class="hcell" role="columnheader"><span class="sr-only">Marker</span></span>
        <span class="hcell" role="columnheader" [attr.aria-sort]="store.descending() ? 'descending' : 'ascending'">
          <button class="sort" (click)="store.toggleSort()">Time {{ store.descending() ? '↓' : '↑' }}</button>
        </span>
        <span class="hcell" role="columnheader">Level</span>
        <span class="hcell" role="columnheader">Instance</span>
        @for (c of store.extraColumns(); track c) {
          <span class="hcell extra" role="columnheader"><span class="ellipsis mono">{{ c }}</span><button class="x" [attr.aria-label]="'Remove column ' + c" (click)="store.removeColumn(c)" [attr.title]="'Remove column ' + c">×</button></span>
        }
        <span class="hcell" role="columnheader">Message</span>
      </div>
      <div class="layer" role="presentation" [style.height.px]="viewportH()">
        @for (v of visible(); track v.index) {
          @if (v.row; as r) {
            <div
              class="row"
              role="row"
              [attr.aria-rowindex]="v.index + 2"
              [attr.aria-selected]="r.id === store.selected()?.id"
              [class.sel]="r.id === store.selected()?.id"
              [style.top.px]="v.y"
              [style.grid-template-columns]="columns()"
              (click)="select(v.index, r)"
              (contextmenu)="openRowMenu($event, v.index, r)"
            >
              <span class="mk" role="gridcell">
                @if (r.markers.length) { <imt-marker-icon [kind]="r.markers[0]" [color]="markerColor(r.markers[0])" [label]="markerLabel(r.markers[0])" /> }
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
            <div class="row placeholder" role="row" [attr.aria-rowindex]="v.index + 2" [style.top.px]="v.y"><span role="gridcell" class="skeleton" aria-label="Loading"></span></div>
          }
        }
        @if (loaded() && total() === 0) {
          <div class="none faint" role="row" aria-rowindex="2"><span role="gridcell">No events match the current filters.</span></div>
        }
      </div>
      <div aria-hidden="true" [style.height.px]="spacerH()"></div>
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; display: flex; flex-direction: column; background: var(--bg-1); min-height: 0; }
    .bar { height: 32px; flex: none; display: flex; align-items: center; gap: 10px; padding: 0 8px 0 14px; border-bottom: 1px solid var(--border); font-size: 12px; }
    .total { color: var(--text-2); }
    .warn { color: var(--warn-text); }
    .spacer { flex: 1; }
    .anchor { position: relative; }
    .cols { right: 0; top: 30px; width: 280px; display: flex; flex-direction: column; gap: 6px; }
    .col-list { max-height: 300px; overflow: auto; }
    .col-opt { display: flex; align-items: center; gap: 8px; height: 24px; color: var(--text-2); font-size: 12px; }
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
    .row.sel { background: var(--bg-sel); box-shadow: inset 2px 0 0 var(--accent); }
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
  protected readonly rowH = computed(() => ROW_HEIGHT[this.settings.density()]);
  private readonly viewport = viewChild<ElementRef<HTMLElement>>('viewport');

  protected readonly fmt = formatCount;
  protected readonly colsOpen = signal(false);
  protected readonly colFilter = signal('');
  protected readonly total = signal(0);
  protected readonly loaded = signal(false);
  protected readonly scrollTop = signal(0);
  protected readonly viewportH = signal(400);
  private readonly cacheTick = signal(0);

  private pages = new Map<number, RecordView[]>();
  private inflight = new Set<number>();
  private generation = 0;
  private query: Query | null = null;
  private selectedIndex = -1;

  protected readonly columns = computed(() => {
    const extra = this.store.extraColumns().map(() => '150px').join(' ');
    return `16px 150px 52px 240px ${extra} minmax(0, 1fr)`;
  });

  protected readonly colCount = computed(() => 5 + this.store.extraColumns().length);

  protected readonly colChoices = computed(() => {
    const needle = this.colFilter().toLowerCase();
    return ['@source', '@line', '@template', ...this.store.fieldNames()].filter((f) => f.toLowerCase().includes(needle));
  });

  private readonly virtualH = computed(() => this.total() * this.rowH());
  protected readonly physicalH = computed(() => Math.min(this.virtualH(), MAX_PX));
  protected readonly spacerH = computed(() => Math.max(0, this.physicalH() - this.viewportH()));
  private readonly ratio = computed(() => {
    const vh = this.viewportH();
    const phys = this.physicalH();
    return phys > vh ? (this.virtualH() - vh) / (phys - vh) : 1;
  });

  protected readonly visible = computed<VisibleRow[]>(() => {
    this.cacheTick();
    const total = this.total();
    const vTop = this.scrollTop() * this.ratio();
    const first = Math.max(0, Math.floor(vTop / this.rowH()) - OVERSCAN);
    const last = Math.min(total, Math.ceil((vTop + this.viewportH()) / this.rowH()) + OVERSCAN);
    const rows: VisibleRow[] = [];
    for (let i = first; i < last; i++) rows.push({ index: i, y: i * this.rowH() - vTop, row: this.rowAt(i) });
    return rows;
  });

  constructor() {
    const destroyRef = inject(DestroyRef);

    // New query, sort order or data: drop the cache and start from the top.
    effect(() => {
      const query = this.store.effectiveQuery();
      const descending = this.store.descending();
      this.store.dataVersion();
      untracked(() => this.reset(query, descending));
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
      if (!anchor || anchor.nonce === handledAnchorNonce) return;
      handledAnchorNonce = anchor.nonce;
      untracked(() => void this.jumpTo(anchor.id));
    });

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
    this.pages = new Map();
    this.inflight.clear();
    this.selectedIndex = -1;
    this.loaded.set(false);
    const el = this.viewport()?.nativeElement;
    if (el) el.scrollTop = 0;
    this.scrollTop.set(0);
    this.cacheTick.update((t) => t + 1);
    void this.fetchPage(0, descending);
  }

  private ensurePage(page: number): void {
    if (this.pages.has(page) || this.inflight.has(page)) return;
    void this.fetchPage(page, this.store.descending());
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
      this.total.set(res.total);
      this.loaded.set(true);
      this.store.setMatchTotal(res.total);
      this.cacheTick.update((t) => t + 1);
    } catch (e) {
      if (gen === this.generation) this.store.setError(String(e));
    } finally {
      this.inflight.delete(page);
    }
  }

  private async jumpTo(id: number): Promise<void> {
    const query = this.query ?? this.store.effectiveQuery();
    const gen = this.generation;
    const res = await this.backend.queryPage({ query, offset: 0, limit: 1, descending: this.store.descending(), anchor: id });
    if (gen !== this.generation) return;
    if (res.anchorIndex === null || !res.rows.length) {
      this.store.anchorResolved(null);
      return;
    }
    this.total.set(res.total);
    this.store.anchorResolved(res.rows[0]);
    this.selectedIndex = res.anchorIndex;
    this.scrollToIndex(res.anchorIndex, true);
  }

  private scrollToIndex(index: number, center: boolean): void {
    const el = this.viewport()?.nativeElement;
    if (!el) return;
    const vh = this.viewportH();
    const vTop = this.scrollTop() * this.ratio();
    const ROW = this.rowH();
    const y = index * ROW;
    let target: number | null = null;
    if (center) target = y - vh / 2 + ROW / 2;
    else if (y < vTop) target = y;
    else if (y + ROW > vTop + vh) target = y + ROW - vh;
    if (target === null) return;
    const scroll = Math.max(0, target) / this.ratio();
    el.scrollTop = scroll;
    this.scrollTop.set(el.scrollTop);
  }

  protected onScroll(): void {
    this.scrollTop.set(this.viewport()?.nativeElement.scrollTop ?? 0);
  }

  protected select(index: number, row: RecordView): void {
    this.selectedIndex = index;
    this.store.select(row);
  }

  protected openRowMenu(e: MouseEvent, index: number, row: RecordView): void {
    e.preventDefault();
    this.select(index, row);
    this.actions.openFor(e, row);
  }

  protected onKey(e: KeyboardEvent): void {
    const total = this.total();
    if (!total) return;
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      const selected = this.store.selected();
      if (selected) {
        e.preventDefault();
        const rowEl = this.viewport()?.nativeElement.querySelector<HTMLElement>('.row.sel') ?? (e.currentTarget as HTMLElement);
        this.actions.openAtElement(rowEl, selected, {}, e.currentTarget as HTMLElement);
      }
      return;
    }
    const pageRows = Math.max(1, Math.floor(this.viewportH() / this.rowH()) - 1);
    const steps: Record<string, number> = { ArrowDown: 1, ArrowUp: -1, PageDown: pageRows, PageUp: -pageRows };
    let next: number;
    if (e.key in steps) next = this.selectedIndex + steps[e.key];
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = total - 1;
    else return;
    e.preventDefault();
    next = Math.max(0, Math.min(total - 1, next));
    this.selectedIndex = next;
    this.scrollToIndex(next, false);
    const row = this.rowAt(next);
    if (row) this.store.select(row);
    else {
      const page = Math.floor(next / PAGE);
      this.ensurePage(page);
      const wait = setInterval(() => {
        const r = this.rowAt(next);
        if (r || this.selectedIndex !== next) {
          clearInterval(wait);
          if (r && this.selectedIndex === next) this.store.select(r);
        }
      }, 50);
    }
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
