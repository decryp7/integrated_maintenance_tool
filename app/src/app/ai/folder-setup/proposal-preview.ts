import { formatCount } from '../../shared/format/units';
import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { LanePreview, ProposalPreview } from './folder-setup.types';

/** Most lanes drawn in the mini lane preview. */
const LANES_SHOWN = 8;

/** What the engine measured for one proposal on the sampled events (counts, lanes, before/after examples). */
@Component({
  selector: 'imt-folder-setup-preview',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @let p = preview();
    @switch (p.kind) {
      @case ('lanes') {
        <p class="facts"><ng-container i18n="@@ai.folderSetup.preview.lanes">{p.totalLanes, plural, =1 {1 lane} other {{{ p.totalLanes }} lanes}} on the sample</ng-container>@if (p.unassigned) {<ng-container i18n="@@ai.folderSetup.preview.unassigned">, {{ p.unassigned }} events without a value</ng-container>}</p>
        <ul class="lanes" data-testid="lane-preview" aria-label="Lanes on the sample" i18n-aria-label="@@ai.folderSetup.preview.lanesLabel">
          @for (l of lanes(); track l.name) {
            <li>
              <span class="lane-name mono" [title]="l.name">{{ l.name }}</span>
              <span class="bar" aria-hidden="true"><span class="fill" [style.width.%]="share(l)"></span></span>
              <span class="count">{{ fmt(l.count) }}</span>
            </li>
          }
        </ul>
        @if (p.totalLanes > lanes().length) {
          <p class="facts" i18n="@@ai.folderSetup.preview.moreLanes">and {{ p.totalLanes - lanes().length }} more</p>
        }
      }
      @case ('matches') {
        <p class="facts" i18n="@@ai.folderSetup.preview.matches">Matches {{ p.matches }} of {{ p.sampled }} sampled events</p>
        <ul class="examples">@for (e of p.examples; track $index) {<li class="mono">{{ e }}</li>}</ul>
      }
      @case ('extracted') {
        <p class="facts" i18n="@@ai.folderSetup.preview.extracted">Extracts values from {{ p.matches }} of {{ p.sampled }} sampled events</p>
        <ul class="examples">@for (e of p.examples; track $index) {<li class="mono">{{ values(e) }}</li>}</ul>
      }
      @case ('runs') {
        <p class="facts" i18n="@@ai.folderSetup.preview.runs">{p.runs, plural, =1 {1 run} other {{{ p.runs }} runs}} on {{ p.lanes }} lanes: {{ p.closed }} closed, {{ p.open }} still running, {{ p.noStop }} without a stop</p>
      }
      @case ('pipeline') {
        <p class="facts" i18n="@@ai.folderSetup.preview.pipeline">Of {{ p.sampled }} sampled events: keeps {{ p.kept }}, drops {{ p.dropped }}, changes {{ p.changed }}</p>
        <ul class="examples">
          @for (e of p.examples; track $index) {
            <li class="mono">
              <span class="before">{{ e.before }}</span>
              @if (e.after) {
                → <span>{{ e.after }}</span>
              } @else {
                <span class="muted" i18n="@@ai.folderSetup.preview.dropped"> (dropped)</span>
              }
            </li>
          }
        </ul>
      }
      @case ('chart') {
        <p class="facts" i18n="@@ai.folderSetup.preview.chart">{{ p.points }} sampled events in {p.series.length, plural, =1 {1 series} other {{{ p.series.length }} series}}: {{ p.series.join(', ') }}</p>
      }
      @case ('dashboard') {
        <p class="facts" i18n="@@ai.folderSetup.preview.dashboard">{p.tiles, plural, =1 {1 tile} other {{{ p.tiles }} tiles}}</p>
      }
      @case ('query') {
        <p class="facts" i18n="@@ai.folderSetup.preview.query">Finds {p.matches, plural, =1 {1 event} other {{{ p.matches }} events}} now</p>
      }
      @case ('parse') {
        <p class="facts" i18n="@@ai.folderSetup.preview.parse">Reads {{ fmt(p.parsed) }} events from {{ fmt(p.lines) }} sample lines, {{ fmt(p.withTimestamp) }} with a timestamp</p>
        <ul class="examples">@for (e of p.examples; track $index) {<li class="mono">{{ e }}</li>}</ul>
      }
    }
  `,
  styles: `
    :host { display: block; }
    .facts { margin: 0; font-size: var(--fs-sm); color: var(--text-2); }
    .examples { margin: 2px 0 0; padding-left: 16px; font-size: var(--fs-xs); color: var(--text-2); }
    .examples li { overflow-wrap: anywhere; }
    .before { color: var(--muted); }
    .lanes { list-style: none; margin: 4px 0 0; padding: 0; display: grid; gap: 2px; max-width: 420px; }
    .lanes li { display: grid; grid-template-columns: minmax(80px, 160px) 1fr auto; gap: 8px; align-items: center; font-size: var(--fs-xs); }
    .lane-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .bar { height: 8px; background: var(--bg-2); border-radius: 4px; overflow: hidden; }
    .fill { display: block; height: 100%; background: var(--accent); }
    .count { color: var(--text-2); font-variant-numeric: tabular-nums; }
  `,
})
export class FolderSetupPreview {
  protected readonly fmt = formatCount;
  readonly preview = input.required<ProposalPreview>();

  protected readonly lanes = computed(() => {
    const p = this.preview();
    return p.kind === 'lanes' ? p.lanes.slice(0, LANES_SHOWN) : [];
  });
  private readonly max = computed(() => Math.max(1, ...this.lanes().map((l) => l.count)));

  protected share(l: LanePreview): number {
    return Math.round((l.count / this.max()) * 100);
  }

  protected values(e: Record<string, unknown>): string {
    return Object.entries(e)
      .map(([k, v]) => `${k}=${typeof v === 'string' ? v : JSON.stringify(v)}`)
      .join(', ');
  }
}
