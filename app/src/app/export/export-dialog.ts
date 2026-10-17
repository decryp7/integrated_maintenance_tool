import { ChangeDetectionStrategy, Component, computed, effect, inject, signal, untracked } from '@angular/core';
import { ExporterInfo, Query } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { formatCount } from '../core/format';
import { Platform } from '../core/platform';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { DIALOG_REF } from '../ui/dialog.service';
import { HelpHint } from '../ui/help-hint';
import { exporterForPath } from './exporters';

/** What to export: the events matching the filters, the same without the time range, or everything. */
export type ExportScope = 'filter' | 'filterAnyTime' | 'all';

/** Builds the query for a scope from the current effective query. */
export function exportQuery(current: Query, scope: ExportScope): Query {
  switch (scope) {
    case 'all':
      return { ...EMPTY_QUERY, groupBy: current.groupBy };
    case 'filterAnyTime':
      return { ...current, from: null, to: null };
    default:
      return current;
  }
}

@Component({
  selector: 'imt-export-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [HelpHint],
  template: `
    <div class="field">
      <label for="exp-format">Format</label>
      <select id="exp-format" class="select" [value]="format()" (change)="format.set($any($event.target).value)">
        @for (e of exporters(); track e.id) {
          <option [value]="e.id">{{ e.name }} (.{{ e.extension }})</option>
        }
      </select>
    </div>

    <fieldset class="field">
      <legend>Which events <imt-help label="Which events" text="'Current filter' exports exactly what the grid shows. 'All events' ignores every filter and the time range." /></legend>
      <label class="opt"><input type="radio" name="scope" value="filter" [checked]="scope() === 'filter'" (change)="scope.set('filter')" /> Current filter</label>
      @if (hasRange()) {
        <label class="opt"><input type="radio" name="scope" value="filterAnyTime" [checked]="scope() === 'filterAnyTime'" (change)="scope.set('filterAnyTime')" /> Current filter, any time (ignore the zoomed range)</label>
      }
      <label class="opt"><input type="radio" name="scope" value="all" [checked]="scope() === 'all'" (change)="scope.set('all')" /> All events</label>
    </fieldset>

    <fieldset class="field">
      <legend>Order</legend>
      <label class="opt"><input type="radio" name="order" value="asc" [checked]="!descending()" (change)="descending.set(false)" /> Oldest first</label>
      <label class="opt"><input type="radio" name="order" value="desc" [checked]="descending()" (change)="descending.set(true)" /> Newest first</label>
    </fieldset>

    <p class="count" role="status" aria-live="polite">
      @if (count() === null) { Counting events… } @else { <strong>{{ fmt(count()!) }}</strong> events will be exported. }
    </p>
    @if (error(); as e) { <p class="err" role="alert">{{ e }}</p> }

    <div class="footer">
      <button class="btn" type="button" (click)="ref.close()">Cancel</button>
      <button class="btn primary" type="button" [disabled]="busy() || !count()" (click)="export()">{{ busy() ? 'Exporting…' : 'Export…' }}</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 12px; }
    .field { display: flex; flex-direction: column; gap: 6px; border: 0; margin: 0; padding: 0; min-width: 0; }
    label, legend { font-size: 12px; color: var(--muted); display: flex; align-items: center; gap: 6px; padding: 0; }
    .opt { color: var(--text-2); font-size: 13px; }
    .count { margin: 0; color: var(--text-2); }
    .err { margin: 0; color: var(--danger-text); }
    .footer { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class ExportDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly store = inject(WorkspaceStore);
  private readonly backend = inject(ImtBackend);
  private readonly platform = inject(Platform);

  protected readonly exporters = computed<ExporterInfo[]>(() => this.store.summary()?.exporters ?? []);
  protected readonly format = signal(this.exporters()[0]?.id ?? '');
  protected readonly scope = signal<ExportScope>('filter');
  protected readonly descending = signal(this.store.descending());
  protected readonly count = signal<number | null>(null);
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly fmt = formatCount;
  protected readonly hasRange = computed(() => this.store.hasRange());

  private readonly query = computed(() => exportQuery(this.store.effectiveQuery(), this.scope()));
  private seq = 0;

  constructor() {
    effect(() => {
      const query = this.query();
      untracked(() => void this.recount(query));
    });
  }

  private async recount(query: Query): Promise<void> {
    const mine = ++this.seq;
    this.count.set(null);
    try {
      const page = await this.backend.queryPage({ query, offset: 0, limit: 1, descending: false, anchor: null });
      if (mine === this.seq) this.count.set(page.total);
    } catch (e) {
      if (mine === this.seq) this.error.set(String(e));
    }
  }

  protected async export(): Promise<void> {
    const exporters = this.exporters();
    const chosen = exporters.find((e) => e.id === this.format()) ?? exporters[0];
    if (!chosen) return;
    this.busy.set(true);
    this.error.set(null);
    try {
      const path = await this.platform.pickExportPath(`events.${chosen.extension}`, [chosen, ...exporters.filter((e) => e !== chosen)]);
      if (!path) return;
      const format = exporterForPath(path, exporters)?.id ?? chosen.id;
      const result = await this.backend.exportEvents({ query: this.query(), format, path, descending: this.descending() });
      this.store.setNotice(`Exported ${formatCount(result.events)} events to ${result.path.split(/[\\/]/).pop()}`);
      this.ref.close();
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    } finally {
      this.busy.set(false);
    }
  }
}
