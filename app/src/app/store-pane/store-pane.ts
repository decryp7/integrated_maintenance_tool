import { Tabs } from '../shared/tabs';
import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, inject, OnInit } from '@angular/core';
import { MaintenanceAction } from '../core/api.types';
import { formatBytes, formatCount, formatDuration, formatEventRate, formatPercent, formatRelativeTime } from '../shared/format/units';
import { Icon } from '../shared/icon';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { DialogService } from '../ui/dialog.service';
import { SqlConsole } from './sql-console';
import { activityLabel, maintenanceText, memoryGauges, storageLabel } from './store-pane-model';
import { SEGMENT_PAGE, STORE_TABS, StorePaneStore, StoreTab } from './store-pane.store';

const TAB_LABELS: Record<StoreTab, string> = {
  overview: $localize`:@@store.tab.overview:Overview`,
  segments: $localize`:@@store.tab.segments:Segments`,
  fields: $localize`:@@store.tab.fields:Fields`,
  index: $localize`:@@store.tab.index:Index`,
  memory: $localize`:@@store.tab.memory:Memory`,
  sql: $localize`:@@store.tab.sql:SQL`,
};

/**
 * View ▸ Store (UX audit C1, ADR 0030): what the window's store holds and how it is doing, read-only, as the engine
 * reports it: overview, segments, fields, full-text index, memory against the budgets, maintenance (confirmed jobs),
 * and the read-only SQL console.
 */
@Component({
  selector: 'imt-store-pane',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [NgTemplateOutlet, SqlConsole, Icon, Tabs],
  template: `
    <div class="tabs">
      <imt-tabs label="Store" i18n-label="@@store.tabs" idPrefix="store-tab-" [items]="tabItems" [selected]="pane.tab()" [panelId]="panelOf" (selectedChange)="pane.show($any($event))" />
      <span class="grow"></span>
      @if (pane.tab() !== 'sql') {
        <button type="button" class="icon-btn" data-testid="store-refresh" aria-label="Refresh" i18n-aria-label="@@store.refresh" title="Refresh" i18n-title="@@store.refresh.title" (click)="pane.refresh()"><imt-icon name="refresh" [size]="14" /></button>
      }
    </div>
    <div class="panel" id="store-panel" role="tabpanel" [attr.aria-labelledby]="'store-tab-' + pane.tab()" tabindex="0">
      @switch (pane.tab()) {
        @case ('overview') {
          @if (pane.overview(); as s) {
            @if (s.error) {
              <ng-container *ngTemplateOutlet="failed; context: { $implicit: s.error }" />
            }
            @if (s.value; as o) {
              <dl class="facts" data-testid="store-overview">
                <dt i18n="@@store.overview.folder">Folder</dt><dd class="mono path">{{ o.dir }}</dd>
                <dt i18n="@@store.overview.size">Size on disk</dt><dd>{{ bytes(o.sizeBytes) }}</dd>
                <dt i18n="@@store.overview.events">Events</dt><dd>{{ count(o.events) }}</dd>
                <dt i18n="@@store.overview.sources">Sources</dt><dd>{{ count(o.sources) }}</dd>
                <dt i18n="@@store.overview.segments">Segments</dt><dd>{{ count(o.segments) }}@if (o.pendingCompaction) {<span class="faint" i18n="@@store.overview.pending"> ({{ o.pendingCompaction }} to compact)</span>}</dd>
                <dt i18n="@@store.overview.state">State</dt><dd data-testid="store-activity">{{ activity(o.activity) }}</dd>
                <dt i18n="@@store.overview.version">Data version</dt><dd>{{ o.dataVersion }} <span class="faint" i18n="@@store.overview.format">(format {{ o.formatVersion }})</span></dd>
                <dt i18n="@@store.overview.lastImport">Last import</dt>
                <dd>
                  @if (o.lastImport; as l) {
                    {{ count(l.events) }} · {{ bytes(l.bytes) }} · {{ duration(l.elapsedMs) }} · {{ rate(l.eventsPerSecond) }} · {{ ago(l.finishedMs) }}
                  } @else {
                    <span class="faint" i18n="@@store.overview.noImport">None yet</span>
                  }
                </dd>
              </dl>
              <h3 class="section-title" i18n="@@store.maintain.title">Maintenance</h3>
              <ng-container *ngTemplateOutlet="maintenance" />
            }
          }
        }
        @case ('segments') {
          @if (pane.segments(); as s) {
            @if (s.error) {
              <ng-container *ngTemplateOutlet="failed; context: { $implicit: s.error }" />
            }
            @if (s.value; as p) {
              <div class="bar">
                <span>{{ segmentsRange() }}</span>
                <span class="grow"></span>
                <button type="button" class="btn" [disabled]="p.offset === 0" (click)="pane.segmentsAt(p.offset - pageSize)" i18n="@@store.segments.prev">Previous</button>
                <button type="button" class="btn" [disabled]="p.offset + p.rows.length >= p.total" (click)="pane.segmentsAt(p.offset + pageSize)" i18n="@@store.segments.next">Next</button>
              </div>
              <div class="table-wrap">
                <table data-testid="store-segments">
                  <thead>
                    <tr>
                      <th scope="col" i18n="@@store.segments.file">File</th>
                      <th scope="col" class="num" i18n="@@store.segments.rows">Rows</th>
                      <th scope="col" class="num" i18n="@@store.segments.size">Size</th>
                      <th scope="col" i18n="@@store.segments.time">Time range (UTC)</th>
                      <th scope="col" i18n="@@store.segments.compression">Compression</th>
                      <th scope="col" class="num" i18n="@@store.segments.sources">Sources</th>
                      <th scope="col" i18n="@@store.segments.indexed">Indexed</th>
                      <th scope="col" i18n="@@store.segments.promoted">Own columns</th>
                      <th scope="col" i18n="@@store.segments.compaction">Compaction</th>
                    </tr>
                  </thead>
                  <tbody>
                    @for (r of p.rows; track r.file) {
                      <tr>
                        <td class="mono">{{ r.file }}</td>
                        <td class="num">{{ count(r.rows) }}@if (r.live < r.rows) {<span class="faint" i18n="@@store.segments.live"> ({{ count(r.live) }} live)</span>}</td>
                        <td class="num">{{ bytes(r.sizeBytes) }}</td>
                        <td>{{ time(r.from) }} – {{ time(r.to) }}</td>
                        <td>{{ r.compression }}</td>
                        <td class="num" [title]="r.sources.join(', ')">{{ count(r.sourceCount) }}</td>
                        <td>{{ r.indexed ? yes : no }}</td>
                        <td class="ellipsis" [title]="r.promoted.join(', ')">{{ r.promoted.length }}</td>
                        <td>{{ r.pendingCompaction ? pendingText : '' }}</td>
                      </tr>
                    }
                  </tbody>
                </table>
              </div>
            }
          }
        }
        @case ('fields') {
          @if (pane.fields(); as s) {
            @if (s.error) {
              <ng-container *ngTemplateOutlet="failed; context: { $implicit: s.error }" />
            }
            @if (s.value; as f) {
              <p class="faint" i18n="@@store.fields.rule">A field gets its own column in a segment when at least half of the segment's first batch has it; filtering on a field in the overflow map is slower.</p>
              <div class="table-wrap">
                <table data-testid="store-fields">
                  <thead>
                    <tr>
                      <th scope="col" i18n="@@store.fields.key">Field</th>
                      <th scope="col" i18n="@@store.fields.type">Type</th>
                      <th scope="col" i18n="@@store.fields.storage">Stored as</th>
                      <th scope="col" class="num" i18n="@@store.fields.fill">Fill rate</th>
                      <th scope="col" class="num" i18n="@@store.fields.distinct">Distinct (approx.)</th>
                    </tr>
                  </thead>
                  <tbody>
                    @for (x of f.fields; track x.key) {
                      <tr>
                        <td class="mono">{{ x.key }}</td>
                        <td>{{ x.type }}</td>
                        <td [title]="x.promotedSegments + ' / ' + x.segments">{{ storage(x.storage) }}</td>
                        <td class="num">{{ pct(x.fillRate * 100) }}</td>
                        <td class="num">≥ {{ count(x.approxDistinct) }}</td>
                      </tr>
                    }
                  </tbody>
                </table>
              </div>
              @if (f.truncated) {
                <p class="faint" i18n="@@store.fields.truncated">The rarest fields are not listed.</p>
              }
            }
          }
        }
        @case ('index') {
          @if (pane.index(); as s) {
            @if (s.error) {
              <ng-container *ngTemplateOutlet="failed; context: { $implicit: s.error }" />
            }
            @if (s.value; as i) {
              <dl class="facts" data-testid="store-index">
                <dt i18n="@@store.index.state">State</dt>
                <dd>@if (i.catchingUp) {<ng-container i18n="@@store.index.catchingUp">Catching up: {{ i.pendingSegments }} segments to index</ng-container>} @else {<ng-container i18n="@@store.index.upToDate">Up to date</ng-container>}</dd>
                <dt i18n="@@store.index.docs">Events indexed</dt><dd>{{ count(i.docs) }} / {{ count(i.events) }}</dd>
                <dt i18n="@@store.index.size">Size</dt><dd>{{ bytes(i.sizeBytes) }}</dd>
                <dt i18n="@@store.index.segments">Index segments</dt><dd>{{ i.indexSegments }}</dd>
                <dt i18n="@@store.index.commit">Last commit</dt><dd>{{ i.lastCommitMs === null ? '—' : ago(i.lastCommitMs) }}</dd>
                <dt i18n="@@store.index.writer">Writer</dt><dd i18n="@@store.index.writerValue">{{ bytes(i.writerHeapBytes) }} heap, {{ i.writerThreads }} threads</dd>
              </dl>
              @if (i.error) {
                <p class="error" role="alert">{{ i.error }}</p>
              }
            }
          }
        }
        @case ('memory') {
          @if (pane.memory(); as s) {
            @if (s.error) {
              <ng-container *ngTemplateOutlet="failed; context: { $implicit: s.error }" />
            }
            @if (s.value; as m) {
              <div class="gauges" data-testid="store-memory">
                @for (g of gauges(); track g.label) {
                  <div class="gauge">
                    <div class="gauge-label"><span>{{ g.label }}</span><span>{{ bytes(g.value) }} / {{ bytes(g.max) }}</span></div>
                    <div class="track" role="meter" [attr.aria-label]="g.label" aria-valuemin="0" [attr.aria-valuemax]="g.max" [attr.aria-valuenow]="g.value"><div class="fill" [class.near]="g.near" [style.width.%]="g.percent"></div></div>
                  </div>
                }
              </div>
              <dl class="facts">
                <dt i18n="@@store.memory.budgets">Budgets</dt><dd i18n="@@store.memory.budgetsValue">{{ bytes(m.importBudgetBytes) }} importing, {{ bytes(m.queryBudgetBytes) }} querying</dd>
                <dt i18n="@@store.memory.duckLimit">DuckDB limit</dt><dd>{{ m.duckLimit }} · <ng-container i18n="@@store.memory.threads">{{ m.duckThreads }} threads</ng-container>@if (m.duckSpilledBytes) {<ng-container i18n="@@store.memory.spilled"> · {{ bytes(m.duckSpilledBytes) }} spilled</ng-container>}</dd>
                <dt i18n="@@store.memory.console">SQL console limit</dt><dd>{{ bytes(m.consoleLimitBytes) }}</dd>
                <dt i18n="@@store.memory.lowMemory">Low memory mode</dt><dd>{{ m.worker?.lowMemory ? on : off }}</dd>
                @if (m.worker; as w) {
                  <dt i18n="@@store.memory.pid">Worker process</dt><dd>{{ w.pid }}</dd>
                }
                <dt i18n="@@store.memory.cached">Cached analyses</dt><dd>{{ m.cachedAnalyses }} · {{ bytes(m.cachedBytes) }}</dd>
                <dt i18n="@@store.memory.jobs">Running jobs</dt>
                <dd>
                  @for (j of m.runningJobs; track $index) {
                    <div>{{ j.title }} <span class="faint">({{ j.kind }})</span></div>
                  } @empty {
                    <span class="faint" i18n="@@store.memory.noJobs">None</span>
                  }
                </dd>
              </dl>
              <h3 class="section-title" i18n="@@store.maintain.title">Maintenance</h3>
              <ng-container *ngTemplateOutlet="maintenance" />
            }
          }
        }
        @case ('sql') {
          <imt-sql-console />
        }
      }
    </div>

    <ng-template #failed let-error>
      <div class="error" role="alert" data-testid="store-error">
        <span>{{ error }}</span>
        <button type="button" class="btn" (click)="pane.refresh()" i18n="@@store.retry">Retry</button>
      </div>
    </ng-template>

    <ng-template #maintenance>
      <div class="bar">
        @for (a of actions; track a) {
          <button type="button" class="btn" [attr.data-testid]="'store-maintain-' + a" [disabled]="busyReason() !== null || pane.starting() !== null" [title]="busyReason() ?? ''" (click)="maintain(a)">{{ actionLabel(a) }}</button>
        }
      </div>
      @if (busyReason(); as why) {
        <p class="faint">{{ why }}</p>
      }
      @if (pane.notice(); as n) {
        <p [class.error]="n.error" role="status" data-testid="store-notice">{{ n.text }}</p>
      }
    </ng-template>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .tabs { display: flex; align-items: center; gap: 2px; padding: 0 12px; border-bottom: 1px solid var(--border-2); }
    .grow { flex: 1; }
    .panel { flex: 1; min-height: 0; overflow: auto; padding: 12px 16px; display: flex; flex-direction: column; gap: 10px; }
    .panel:focus-visible { outline-offset: -2px; }
    .facts { display: grid; grid-template-columns: 120px 1fr; gap: 6px 16px; margin: 0; font-size: var(--fs-md, 13px); font-variant-numeric: tabular-nums; }
    dt { color: var(--muted); }
    dd { margin: 0; min-width: 0; }
    .path { overflow-wrap: anywhere; font-size: var(--fs-sm); }
    .bar { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .table-wrap { overflow: auto; border: 1px solid var(--border-2); border-radius: 6px; }
    table { border-collapse: collapse; width: 100%; font-size: var(--fs-sm); }
    th, td { text-align: left; padding: 3px 8px; border-bottom: 1px solid var(--border-2); white-space: nowrap; }
    th { position: sticky; top: 0; background: var(--bg-2); font-weight: 600; }
    .num { text-align: right; font-variant-numeric: tabular-nums; }
    .error { color: var(--lvl-error); display: flex; align-items: center; gap: 8px; margin: 0; }
    .gauges { display: flex; flex-direction: column; gap: 10px; max-width: 560px; }
    .gauge-label { display: flex; justify-content: space-between; font-size: var(--fs-sm); }
    .track { height: 8px; border-radius: 4px; background: var(--bg-3, var(--bg-2)); overflow: hidden; }
    .fill { height: 100%; background: var(--accent); }
    .fill.near { background: var(--lvl-warn); }
    h3 { margin: 6px 0 0; }
    p { margin: 0; }
  `,
})
export class StorePane implements OnInit {
  protected readonly pane = inject(StorePaneStore);
  private readonly dialogs = inject(DialogService);
  protected readonly tabs = STORE_TABS;
  protected readonly pageSize = SEGMENT_PAGE;
  protected readonly actions: MaintenanceAction[] = ['compact', 'rebuildIndex', 'clearCachedAnalyses'];
  protected readonly yes = $localize`:@@store.yes:Yes`;
  protected readonly no = $localize`:@@store.no:No`;
  protected readonly on = $localize`:@@store.on:On`;
  protected readonly off = $localize`:@@store.off:Off`;
  protected readonly pendingText = $localize`:@@store.segments.pendingCompaction:Pending`;

  protected readonly gauges = computed(() => {
    const m = this.pane.memory().value;
    return m ? memoryGauges(m) : [];
  });

  /** Why maintenance waits: an import runs (from the last overview or memory report). */
  protected readonly busyReason = computed(() =>
    this.pane.overview().value?.activity === 'importing' || this.pane.memory().value?.importing
      ? $localize`:@@store.maintain.busy:Maintenance waits until the import is done.`
      : null,
  );

  protected readonly segmentsRange = computed(() => {
    const p = this.pane.segments().value;
    if (!p || !p.rows.length) return $localize`:@@store.segments.none:No segments yet.`;
    return $localize`:@@store.segments.range:Segments ${p.offset + 1}:from:–${p.offset + p.rows.length}:to: of ${p.total}:total:`;
  });

  ngOnInit(): void {
    void this.pane.show(this.pane.tab());
  }

  protected label(t: StoreTab): string {
    return TAB_LABELS[t];
  }
  protected actionLabel(a: MaintenanceAction): string {
    return maintenanceText(a).title;
  }

  protected readonly tabItems = STORE_TABS.map((t) => ({ id: t, label: TAB_LABELS[t] }));
  protected readonly panelOf = () => 'store-panel';

  protected async maintain(action: MaintenanceAction): Promise<void> {
    const t = maintenanceText(action);
    const ref = this.dialogs.open<boolean>(ConfirmDialog, { title: t.title, width: 460, inputs: { message: t.message, confirmLabel: t.confirm } });
    if ((await ref.closed) !== true) return;
    await this.pane.maintain(action);
  }

  protected bytes = (n: number) => formatBytes(n);
  protected count = (n: number) => formatCount(n);
  protected duration = (ms: number) => formatDuration(ms);
  protected rate = (n: number) => formatEventRate(n);
  protected pct = (n: number) => formatPercent(n);
  protected ago = (ms: number) => formatRelativeTime(ms, Date.now());
  protected activity = activityLabel;
  protected storage = storageLabel;
  protected time = (ms: number | null) => (ms === null ? '—' : new Date(ms).toISOString().slice(0, 16).replace('T', ' '));
}
