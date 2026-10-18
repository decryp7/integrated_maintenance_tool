import { ChangeDetectionStrategy, Component, computed, effect, inject, input, signal, untracked } from '@angular/core';
import { HeapObjectRow } from '../../core/api.types';
import { formatCount } from '../../core/format';
import { errorText } from '../../analysis/async-resource';
import { ProfilingBackend } from '../profiling-backend';
import { formatBytes } from '../profiling-format';
import { PROFILING_STYLES } from '../profiling-styles';
import { HeapSelection } from './heap-selection';

const PAGE = 100;

interface Level {
  rows: HeapObjectRow[];
  total: number;
  loading: boolean;
  error: string | null;
}

interface DomRow {
  row: HeapObjectRow;
  level: number;
  pos: number;
  size: number;
  expanded: boolean;
}

/** Flattens the loaded dominator levels into visible rows (pure). */
export function flattenDominators(levels: ReadonlyMap<number | null, Level>, expanded: ReadonlySet<number>): DomRow[] {
  const out: DomRow[] = [];
  const walk = (parent: number | null, level: number) => {
    const l = levels.get(parent);
    if (!l) return;
    l.rows.forEach((row, i) => {
      const open = expanded.has(row.object) && row.childCount > 0;
      out.push({ row, level, pos: i + 1, size: l.total, expanded: open });
      if (open) walk(row.object, level + 1);
    });
  };
  walk(null, 1);
  return out;
}

/** The dominator tree ("what keeps what alive"), loaded lazily level by level, as a treegrid. */
@Component({
  selector: 'imt-heap-dominators',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (top().error; as e) {
      <div class="state" role="alert"><span class="err">{{ e }}</span><button class="btn" type="button" (click)="load(null)" i18n="@@profiling.heapDominators.retry">Retry</button></div>
    } @else if (top().loading && !top().rows.length) {
      <div class="state" role="status" i18n="@@profiling.heapDominators.loading">Loading…</div>
    } @else {
      <div class="body tight" tabindex="0" role="region" aria-label="Dominator tree rows" i18n-aria-label="@@profiling.heapDominators.rowsRegion">
        <table class="grid" role="treegrid" aria-label="Dominator tree" i18n-aria-label="@@profiling.heapDominators.tableLabel" (keydown)="onKey($event)">
          <thead><tr role="row"><th role="columnheader" i18n="@@profiling.heapDominators.col.object">Object</th><th role="columnheader" class="num" i18n="@@profiling.heapDominators.col.retained">Retained</th><th role="columnheader" class="num" i18n="@@profiling.heapDominators.col.shallow">Shallow</th><th role="columnheader" class="num" i18n="@@profiling.heapDominators.col.dominates">Dominates</th><th role="columnheader"><span class="sr" i18n="@@profiling.heapDominators.col.actions">Actions</span></th></tr></thead>
          <tbody>
            @for (r of rows(); track r.row.object; let i = $index) {
              <tr role="row" [attr.aria-level]="r.level" [attr.aria-posinset]="r.pos" [attr.aria-setsize]="r.size" [attr.aria-expanded]="r.row.childCount ? r.expanded : null" [attr.tabindex]="focus() === i ? 0 : -1" (click)="focus.set(i)">
                <td role="gridcell"><span class="obj" [style.padding-left.px]="(r.level - 1) * 14">
                  <span class="tw" aria-hidden="true" (click)="toggle(r.row)">{{ r.row.childCount ? (r.expanded ? '▾' : '▸') : '' }}</span>
                  <span class="ellipsis">{{ r.row.typeName }}</span> <span class="faint mono">&#64;{{ r.row.object }}</span>
                </span></td>
                <td role="gridcell" class="num">{{ bytes(r.row.retained) }}</td>
                <td role="gridcell" class="num">{{ bytes(r.row.shallow) }}</td>
                <td role="gridcell" class="num">{{ count(r.row.childCount) }}</td>
                <td role="gridcell"><button class="mini" type="button" [attr.aria-label]="whyLabel(r.row.typeName, r.row.object)" (click)="sel.showPaths(r.row.object, r.row.typeName)" i18n="@@profiling.heapDominators.paths">Paths to roots</button></td>
              </tr>
            }
          </tbody>
        </table>
        @for (p of pending(); track p) { <div class="muted pad">{{ p }}</div> }
      </div>
    }
  `,
  styles: [
    PROFILING_STYLES,
    `
      .tight { padding: 0; }
      .state { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 24px 12px; color: var(--muted); }
      .err { color: var(--danger-text); }
      .obj { display: flex; align-items: center; gap: 4px; min-width: 0; max-width: 520px; }
      .tw { width: 12px; flex: none; cursor: pointer; color: var(--muted); }
      .faint { color: var(--faint); }
      .pad { padding: 4px 12px; }
      tr[tabindex]:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
    `,
  ],
})
export class HeapDominators {
  readonly snapshotId = input.required<number>();
  protected readonly sel = inject(HeapSelection);
  private readonly backend = inject(ProfilingBackend);
  private readonly levels = signal<ReadonlyMap<number | null, Level>>(new Map());
  private readonly expanded = signal<ReadonlySet<number>>(new Set());
  protected readonly focus = signal(0);
  protected readonly count = formatCount;
  protected readonly bytes = formatBytes;
  protected readonly top = computed<Level>(() => this.levels().get(null) ?? { rows: [], total: 0, loading: true, error: null });
  protected readonly rows = computed(() => flattenDominators(this.levels(), this.expanded()));
  /** Loading/error lines of expanded levels. */
  protected readonly pending = computed(() =>
    [...this.levels().entries()].filter(([k, l]) => k !== null && (l.loading || l.error)).map(([k, l]) => (l.error ? `@${k}: ${l.error}` : $localize`:@@profiling.heapDominators.loadingChildren:Loading children of @${k}:object:…`)),
  );
  private generation = 0;

  constructor() {
    effect(() => {
      this.snapshotId();
      untracked(() => {
        this.generation++;
        this.levels.set(new Map());
        this.expanded.set(new Set());
        void this.load(null);
      });
    });
  }

  protected whyLabel(typeName: string, object: number): string {
    return $localize`:@@profiling.heapDominators.whyLabel:Why is ${typeName}:type: @${object}:object: alive?`;
  }

  private setLevel(parent: number | null, l: Level): void {
    const next = new Map(this.levels());
    next.set(parent, l);
    this.levels.set(next);
  }

  async load(parent: number | null): Promise<void> {
    const mine = this.generation;
    this.setLevel(parent, { rows: [], total: 0, loading: true, error: null });
    try {
      const page = await this.backend.heapDominators({ snapshotId: this.snapshotId(), parent, limit: PAGE });
      if (mine === this.generation) this.setLevel(parent, { rows: page.rows, total: page.totalRows, loading: false, error: null });
    } catch (e) {
      if (mine === this.generation) this.setLevel(parent, { rows: [], total: 0, loading: false, error: errorText(e) });
    }
  }

  protected toggle(r: HeapObjectRow): void {
    if (!r.childCount) return;
    const next = new Set(this.expanded());
    if (next.has(r.object)) next.delete(r.object);
    else {
      next.add(r.object);
      // Not loaded yet, or the last attempt failed: (re)load.
      const level = this.levels().get(r.object);
      if (!level || level.error) void this.load(r.object);
    }
    this.expanded.set(next);
  }

  protected onKey(e: KeyboardEvent): void {
    const rows = this.rows();
    const i = this.focus();
    const r = rows[i];
    if (!r) return;
    let next = i;
    if (e.key === 'ArrowDown') next = Math.min(rows.length - 1, i + 1);
    else if (e.key === 'ArrowUp') next = Math.max(0, i - 1);
    else if (e.key === 'ArrowRight' && r.row.childCount && !r.expanded) this.toggle(r.row);
    else if (e.key === 'ArrowLeft' && r.expanded) this.toggle(r.row);
    else if (e.key === 'Enter') this.sel.showPaths(r.row.object, r.row.typeName);
    else return;
    e.preventDefault();
    this.focus.set(next);
    const table = e.currentTarget as HTMLElement;
    queueMicrotask(() => table.querySelectorAll<HTMLElement>('tbody tr')[next]?.focus());
  }
}
