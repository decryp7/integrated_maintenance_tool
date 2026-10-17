import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { ChartType } from '../core/api.types';
import { formatDateTime, tzLabel } from '../core/format';
import { DockService } from '../dock/dock.service';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { TimelineZoom } from '../timeline/zoom';
import { AdhocCharts } from './adhoc-charts';
import { ChartCard } from './chart-card';
import { NextSteps } from '../ui/next-steps';

const AGGREGATES = ['avg', 'max', 'min', 'sum', 'last', 'count'] as const;
const BUCKETS = ['auto', '10s', '30s', '1m', '5m', '15m', '1h'];

@Component({
  selector: 'imt-metrics-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ChartCard, Icon, NextSteps],
  template: `
    <div class="bar">
      <span class="section-title">Metrics</span>
      <span class="faint">{{ store.summary()?.profileName }} · {{ charts().length }} chart{{ charts().length === 1 ? '' : 's' }} · drag across a chart to zoom</span>
      <span class="spacer"></span>
      <span class="mono range">{{ range() }}</span>
      @if (store.hasRange()) {
        <button class="btn" (click)="zoom.fit()"><imt-icon name="zoomOut" [size]="13" /> Reset zoom</button>
      }
      <button class="btn dashed" (click)="builderOpen.set(!builderOpen())" [attr.aria-expanded]="builderOpen()">+ Add chart</button>
      <button class="btn ghost" (click)="dock.show('profile')">Edit charts in profile</button>
    </div>

    @if (builderOpen()) {
      <form class="builder" (submit)="$event.preventDefault(); addChart()">
        <label class="lbl">Title<input class="input" [value]="title()" (input)="title.set($any($event.target).value)" placeholder="e.g. CPU %" /></label>
        <label class="lbl">Aggregate
          <select class="select" [value]="agg()" (change)="agg.set($any($event.target).value)">
            @for (a of aggregates; track a) { <option [value]="a" [selected]="a === agg()">{{ a }}</option> }
          </select>
        </label>
        @if (agg() !== 'count') {
          <label class="lbl">Field
            <select class="select mono" [value]="field()" (change)="field.set($any($event.target).value)">
              <option value="" disabled [selected]="!field()">Choose a numeric field</option>
              @for (f of store.numericFields(); track f) { <option [value]="f" [selected]="f === field()">{{ f }}</option> }
            </select>
          </label>
        }
        <label class="lbl">Bucket
          <select class="select" [value]="bucket()" (change)="bucket.set($any($event.target).value)">
            @for (b of buckets; track b) { <option [value]="b" [selected]="b === bucket()">{{ b }}</option> }
          </select>
        </label>
        <label class="lbl">Split by
          <select class="select mono" multiple size="3" (change)="setSplit($event)">
            @for (f of splitChoices(); track f) { <option [value]="f" [selected]="splitBy().includes(f)">{{ f }}</option> }
          </select>
        </label>
        <label class="lbl">Type
          <select class="select" [value]="type()" (change)="type.set($any($event.target).value)">
            <option value="line">line</option>
            <option value="bar">bar</option>
            <option value="stacked">stacked bars</option>
          </select>
        </label>
        <label class="lbl">Unit<input class="input" [value]="unit()" (input)="unit.set($any($event.target).value)" placeholder="%, MB, ms" /></label>
        <div class="b-actions">
          <button type="button" class="btn ghost" (click)="builderOpen.set(false)">Cancel</button>
          <button type="submit" class="btn primary" [disabled]="agg() !== 'count' && !field()">Add chart</button>
        </div>
      </form>
    }

    <div class="charts" tabindex="0" role="region" aria-label="Charts">
      @for (c of profileCharts(); track c.id) {
        <imt-chart-card [def]="c" />
      }
      @for (c of adhoc(); track c.id) {
        <imt-chart-card [def]="c" [fromProfile]="false" [removable]="true" (remove)="removeChart(c.id)" />
      }
      @if (!charts().length) {
        <div class="empty-state">
          <div>No charts defined.</div>
          <div class="faint">Add [[chart]] sections to the profile, or build one here from any numeric field.</div>
          @if (store.summary()?.sources?.length) {
            <button type="button" class="btn" (click)="dock.show('profile')">Open the profile</button>
          } @else {
            <imt-next-steps />
          }
        </div>
      }
    </div>
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .bar { flex: none; display: flex; align-items: center; gap: 10px; padding: 10px 14px; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
    .spacer { flex: 1; }
    .range { font-size: 12px; color: var(--text-2); }
    .builder { flex: none; display: flex; flex-wrap: wrap; align-items: flex-end; gap: 10px; padding: 12px 14px; border-bottom: 1px solid var(--border); background: var(--bg-2); }
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: 11px; color: var(--muted); }
    select[multiple] { height: auto; padding: 2px; }
    .b-actions { display: flex; gap: 6px; margin-left: auto; }
    .charts { flex: 1; min-height: 0; overflow: auto; }
  `,
})
export class MetricsView {
  protected readonly store = inject(WorkspaceStore);
  protected readonly dock = inject(DockService);
  protected readonly zoom = inject(TimelineZoom);
  protected readonly aggregates = AGGREGATES;
  protected readonly buckets = BUCKETS;
  private readonly adhocCharts = inject(AdhocCharts);
  protected readonly adhoc = this.adhocCharts.charts;

  protected readonly builderOpen = signal(false);
  protected readonly title = signal('');
  protected readonly agg = signal<(typeof AGGREGATES)[number]>('avg');
  protected readonly field = signal('');
  protected readonly bucket = signal('auto');
  protected readonly splitBy = signal<string[]>([]);
  protected readonly type = signal<ChartType>('line');
  protected readonly unit = signal('');

  protected readonly profileCharts = computed(() => this.store.summary()?.charts ?? []);
  protected readonly charts = computed(() => [...this.profileCharts(), ...this.adhoc()]);
  protected readonly splitChoices = computed(() => {
    const fields = (this.store.summary()?.fields ?? []).filter((f) => f.distinct <= 200).map((f) => f.name);
    return ['@level', '@source', ...fields];
  });
  protected readonly range = computed(() => {
    const tz = this.store.timezone();
    const q = this.store.query();
    const s = this.store.summary();
    return `${formatDateTime(q.from ?? s?.from ?? null, tz)} → ${formatDateTime(q.to ?? s?.to ?? null, tz)} ${tzLabel(tz)}`;
  });

  protected setSplit(e: Event): void {
    const sel = e.target as HTMLSelectElement;
    this.splitBy.set([...sel.selectedOptions].map((o) => o.value));
  }

  protected addChart(): void {
    const agg = this.agg();
    const y = agg === 'count' ? 'count()' : `${agg}(${this.field()})`;
    this.adhocCharts.add({
      title: this.title() || y,
      y,
      bucket: this.bucket(),
      splitBy: this.splitBy(),
      type: this.type(),
      unit: this.unit(),
    });
    this.builderOpen.set(false);
    this.title.set('');
  }

  protected removeChart(id: string): void {
    this.adhocCharts.remove(id);
  }
}
