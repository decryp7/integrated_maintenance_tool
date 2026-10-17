import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { cellText, downloadText, SortDir, sortRows, toCsv } from './table-utils';
import { QuerySession } from './query-session';

/** The table produced by a `stats` query: sortable columns and a client-side CSV export. */
@Component({
  selector: 'imt-query-results-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="bar">
      <span class="section-title">Query results</span>
      @if (table(); as t) {
        <span class="faint">{{ t.rows.length }} row{{ t.rows.length === 1 ? '' : 's' }} · {{ t.columns.length }} column{{ t.columns.length === 1 ? '' : 's' }}</span>
      }
      <span class="spacer"></span>
      <button class="btn" type="button" [disabled]="!table()" (click)="exportCsv()">Export CSV</button>
    </div>
    @if (table(); as t) {
      <div class="body" tabindex="0" role="region" aria-label="Query result table">
        <table class="grid">
          <caption class="sr">Result of the last stats query</caption>
          <thead>
            <tr>
              @for (c of t.columns; track $index; let i = $index) {
                <th scope="col" [attr.aria-sort]="ariaSort(i)">
                  <button type="button" (click)="sortBy(i)">{{ c }}<span aria-hidden="true">{{ marker(i) }}</span></button>
                </th>
              }
            </tr>
          </thead>
          <tbody>
            @for (row of rows(); track $index) {
              <tr>
                @for (cell of row; track $index) {
                  <td [class.num]="isNumber(cell)">{{ text(cell) }}</td>
                }
              </tr>
            }
          </tbody>
        </table>
      </div>
    } @else {
      <div class="empty" role="status">Run a query with a <code class="mono">stats</code> stage (for example <code class="mono">stats count() by source</code>) to see its table here.</div>
    }
  `,
  styles: [
    `
      :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); color: var(--text-2); font-size: 12px; }
      .bar { flex: none; display: flex; align-items: center; gap: 10px; padding: 6px 12px; border-bottom: 1px solid var(--border); }
      .spacer { flex: 1; }
      .body { flex: 1; min-height: 0; overflow: auto; }
      .empty { padding: 28px 16px; color: var(--muted); text-align: center; }
      .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }
      table.grid { border-collapse: collapse; width: 100%; }
      table.grid th, table.grid td { padding: 4px 10px; border-bottom: 1px solid var(--border); text-align: left; white-space: nowrap; }
      table.grid th { position: sticky; top: 0; background: var(--bg-2); color: var(--muted); font-weight: 600; }
      table.grid th button { all: unset; cursor: pointer; display: inline-flex; gap: 4px; }
      table.grid th button:focus-visible { outline: 2px solid var(--accent); }
      table.grid td.num { text-align: right; font-family: var(--font-mono); font-variant-numeric: tabular-nums; }
    `,
  ],
})
export class QueryResultsView {
  private readonly session = inject(QuerySession);
  protected readonly table = this.session.table;
  protected readonly sort = signal<{ col: number; dir: SortDir } | null>(null);
  protected readonly rows = computed(() => {
    const t = this.table();
    const s = this.sort();
    if (!t) return [];
    return s ? sortRows(t.rows, s.col, s.dir) : t.rows;
  });
  protected readonly text = cellText;

  protected isNumber(v: unknown): boolean {
    return typeof v === 'number';
  }

  protected sortBy(col: number): void {
    const s = this.sort();
    this.sort.set(s?.col === col ? { col, dir: s.dir === 'asc' ? 'desc' : 'asc' } : { col, dir: 'asc' });
  }

  protected ariaSort(col: number): 'ascending' | 'descending' | null {
    const s = this.sort();
    return s?.col === col ? (s.dir === 'asc' ? 'ascending' : 'descending') : null;
  }

  protected marker(col: number): string {
    const s = this.sort();
    return s?.col === col ? (s.dir === 'asc' ? '▲' : '▼') : '';
  }

  protected exportCsv(): void {
    const t = this.table();
    if (t) downloadText('query-result.csv', toCsv(t.columns, this.rows()));
  }
}
