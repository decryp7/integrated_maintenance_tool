import { ChangeDetectionStrategy, Component, computed, effect, inject, input, signal, untracked } from '@angular/core';
import { SparkPoint, TileCfg, TileData } from '../../core/api.types';
import { formatCount, formatDateTime, formatNumber } from '../../core/format';
import { WorkspaceStore } from '../../core/workspace.store';
import { buildOption } from '../../metrics/chart-card';
import { ChartTheme } from '../../metrics/chart-theme';
import { EChart } from '../../metrics/echart';
import { ShareBackend } from '../share-backend';

/** Polyline points of a sparkline in a 100×24 box; null values are skipped. */
export function sparkPoints(points: readonly SparkPoint[]): string {
  const values = points.map((p) => p[1]).filter((v): v is number => v !== null);
  if (values.length < 2 && points.length < 2) return '';
  const max = Math.max(...values, 0);
  const min = Math.min(...values, 0);
  const span = max - min || 1;
  const step = points.length > 1 ? 100 / (points.length - 1) : 0;
  return points
    .map((p, i) => (p[1] === null ? null : `${(i * step).toFixed(1)},${(22 - ((p[1] - min) / span) * 20).toFixed(1)}`))
    .filter((s): s is string => s !== null)
    .join(' ');
}

/** A table cell as text (`@t` columns are timestamps). */
export function cellText(column: string, value: unknown, tz: 'utc' | 'local'): string {
  if (value === null || value === undefined) return '';
  if (column === '@t' && typeof value === 'number') return formatDateTime(value, tz);
  if (typeof value === 'number') return formatNumber(value);
  return typeof value === 'string' ? value : JSON.stringify(value);
}

/** One dashboard tile: asks the engine for its data (again when the query or data change) and draws it. */
@Component({
  selector: 'imt-dashboard-tile',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [EChart],
  template: `
    @if (error(); as e) {
      <div class="msg err" role="alert">
        <span>{{ e }}</span>
        <button type="button" class="btn" (click)="retry()" i18n="@@share.dashboardTile.retry">Retry</button>
      </div>
    } @else if (data(); as d) {
      @switch (d.kind) {
        @case ('stat') {
          <div class="stat">
            <span class="value" data-testid="stat-value">{{ statText() }}@if (d.unit) {<span class="unit"> {{ d.unit }}</span>}</span>
            <span class="label faint">{{ d.label }}</span>
            @if (spark(); as pts) {
              <svg class="spark" viewBox="0 0 100 24" preserveAspectRatio="none" aria-hidden="true"><polyline [attr.points]="pts" /></svg>
            }
          </div>
        }
        @case ('chart') {
          @if (d.data.series.length) {
            <imt-echart class="chart" [options]="chartOption()!" />
          } @else {
            <div class="msg faint" i18n="@@share.dashboardTile.noData">No data in this range.</div>
          }
        }
        @case ('table') {
          <div class="scroll" tabindex="0" [attr.aria-label]="name()">
            <table>
              <thead><tr>@for (c of d.columns; track $index) {<th scope="col">{{ c }}</th>}</tr></thead>
              <tbody>
                @for (row of d.rows; track $index) {
                  <tr>@for (c of d.columns; track $index; let i = $index) {<td>{{ cell(c, row[i]) }}</td>}</tr>
                }
              </tbody>
            </table>
            @if (!d.rows.length) {<div class="msg faint" i18n="@@share.dashboardTile.noRows">No rows.</div>}
            @if (d.truncated) {<div class="faint more" i18n="@@share.dashboardTile.moreRows">More rows not shown.</div>}
          </div>
        }
        @case ('top') {
          <ul class="list top" tabindex="0" [attr.aria-label]="name()">
            @for (v of d.facet.values; track v.value) {
              <li>
                <span class="bar" [style.width.%]="share(v.count)" aria-hidden="true"></span>
                <span class="name" [title]="v.value">{{ v.value }}</span>
                <span class="count mono">{{ count(v.count) }}</span>
              </li>
            } @empty {
              <li class="faint" i18n="@@share.dashboardTile.noValues">No values.</li>
            }
          </ul>
        }
        @case ('alerts') {
          <ul class="list" tabindex="0" [attr.aria-label]="name()">
            @for (a of d.items; track a.kind + ':' + a.instance + ':' + a.from + ':' + a.to) {
              <li><div class="meta"><span class="kind">{{ a.kind }}</span><span class="faint mono">{{ time(a.from) }}</span>@if (a.instance) {<span class="faint">· {{ a.instance }}</span>}</div><div class="detail">{{ a.message }}</div></li>
            } @empty {
              <li class="faint" i18n="@@share.dashboardTile.noAlerts">No alerts.</li>
            }
          </ul>
        }
        @case ('annotations') {
          <ul class="list" tabindex="0" [attr.aria-label]="name()">
            @for (a of d.items; track a.label + ':' + a.from + ':' + a.to) {
              <li><span class="kind">{{ a.label }}</span> <span class="faint mono">{{ time(a.from) }}@if (a.to !== a.from) { – {{ time(a.to) }}}</span></li>
            } @empty {
              <li class="faint" i18n="@@share.dashboardTile.noAnnotations">No annotations.</li>
            }
          </ul>
        }
      }
    } @else {
      <div class="msg faint" role="status" i18n="@@share.dashboardTile.loading">Loading…</div>
    }
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; }
    .msg { flex: 1; display: flex; flex-direction: column; gap: 8px; align-items: center; justify-content: center; text-align: center; padding: 8px; font-size: var(--fs-sm); }
    .err { color: var(--danger-text); }
    .stat { flex: 1; display: flex; flex-direction: column; justify-content: center; gap: 2px; min-height: 0; }
    .value { font-size: var(--tile-value-size, 28px); font-weight: 600; font-variant-numeric: tabular-nums; line-height: 1.1; }
    .unit { font-size: 0.5em; font-weight: 400; color: var(--muted); }
    .label { font-size: var(--fs-xs); }
    .spark { width: 100%; height: 28px; margin-top: 4px; }
    .spark polyline { fill: none; stroke: var(--accent); stroke-width: 1.5; vector-effect: non-scaling-stroke; }
    .chart { flex: 1; min-height: 100px; }
    .scroll { flex: 1; min-height: 0; overflow: auto; }
    table { width: 100%; border-collapse: collapse; font-size: var(--fs-sm); }
    th, td { text-align: left; padding: 3px 6px; border-bottom: 1px solid var(--border); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 260px; }
    th { color: var(--muted); font-weight: 600; position: sticky; top: 0; background: var(--bg-2); }
    .more { font-size: var(--fs-xs); padding: 4px 6px; }
    .list { list-style: none; margin: 0; padding: 0; overflow: auto; flex: 1; min-height: 0; font-size: var(--fs-sm); display: flex; flex-direction: column; gap: 4px; }
    .top li { position: relative; display: flex; gap: 8px; padding: 2px 4px; }
    .top .bar { position: absolute; inset: 0 auto 0 0; background: var(--accent-soft); border-radius: 2px; }
    .top .name, .top .count { position: relative; }
    .top .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .kind { font-weight: 600; }
    .meta { display: flex; flex-wrap: wrap; gap: 0 8px; align-items: baseline; }
    .detail { color: var(--text-2); }
  `,
})
export class DashboardTile {
  readonly tile = input.required<TileCfg>();

  private readonly backend = inject(ShareBackend);
  private readonly store = inject(WorkspaceStore);
  private readonly theme = inject(ChartTheme);

  protected readonly data = signal<TileData | null>(null);
  protected readonly error = signal<string | null>(null);
  private readonly attempt = signal(0);

  protected readonly statText = computed(() => {
    const d = this.data();
    if (d?.kind !== 'stat') return '';
    if (d.value !== null) return formatNumber(d.value);
    return d.text || '—';
  });

  protected readonly spark = computed(() => {
    const d = this.data();
    return d?.kind === 'stat' ? sparkPoints(d.sparkline) || null : null;
  });

  protected readonly chartOption = computed(() => {
    const d = this.data();
    if (d?.kind !== 'chart') return null;
    const labels = new Map([...this.store.markerKindMap().values()].map((k) => [k.kind, k.label]));
    return buildOption(d.data, this.theme.tokens(), this.store.timezone(), labels);
  });

  private readonly maxCount = computed(() => {
    const d = this.data();
    return d?.kind === 'top' ? Math.max(1, ...d.facet.values.map((v) => v.count)) : 1;
  });

  constructor() {
    let seq = 0;
    effect(() => {
      const tile = this.tile();
      const query = this.store.effectiveQuery();
      this.store.dataVersion();
      this.attempt();
      untracked(() => {
        const mine = ++seq;
        this.backend
          .dashboardTile({ tile, query })
          .then((d) => {
            if (mine !== seq) return;
            this.data.set(d);
            this.error.set(null);
          })
          .catch((e) => {
            if (mine !== seq) return;
            this.error.set(e instanceof Error ? e.message : String(e));
          });
      });
    });
  }

  retry(): void {
    this.error.set(null);
    this.data.set(null);
    this.attempt.update((n) => n + 1);
  }

  protected name(): string {
    const t = this.tile();
    return t.title || t.id;
  }

  protected cell(column: string, value: unknown): string {
    return cellText(column, value, this.store.timezone());
  }

  protected share(count: number): number {
    return Math.round((count / this.maxCount()) * 100);
  }

  protected count(n: number): string {
    return formatCount(n);
  }

  protected time(ts: number): string {
    return formatDateTime(ts, this.store.timezone());
  }
}
