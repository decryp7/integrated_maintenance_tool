import { ChangeDetectionStrategy, Component, effect, inject, signal, untracked } from '@angular/core';
import { formatCount, formatDuration, formatNumber } from '../core/format';
import { Comparison, FieldShift, SideStats } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { barWidth, direction, ARROWS, formatChange } from './analysis-format';
import { ImtSelect, SelectOption } from '../settings/themed-select';
import { AnalysisState } from './analysis-state';
import { ANALYSIS_STYLES } from './analysis-styles';
import { AnalysisTargets, windowQuery } from './analysis-targets';
import { AnalysisTime } from './analysis-time';
import { AnalysisResource } from './async-resource';
import { CoverageNote } from './coverage-note';
import { TimeCursor } from './time-cursor';
import { aroundTime, midpoint, previousWindow, WIDTHS } from './windows';

/** Values listed per changed field before "Show all". */
const TOP_VALUES = 5;

/** Two time windows side by side: counts, which fields changed, numeric shifts, new and gone values. */
@Component({
  selector: 'imt-compare-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, CoverageNote, ImtSelect],
  template: `
    <div class="bar">
      <span class="chip-ab a" data-testid="window-a"><span class="sw" aria-hidden="true"></span>A: <span class="nums">{{ time.range(targets.windowA()) }}</span></span>
      <span class="chip-ab b" data-testid="window-b"><span class="sw" aria-hidden="true"></span>B: <span class="nums">{{ time.range(targets.windowB()) }}</span></span>
      <span class="spacer"></span>
      <button class="btn" type="button" (click)="previous()" i18n="@@analysis.compare.currentVsPrevious">Current range vs previous</button>
      <span class="lbl"><ng-container i18n="@@analysis.compare.window">Window</ng-container>
        <imt-select [options]="widthOptions" [value]="width()" (valueChange)="width.set(+$event)" ariaLabel="Window length for before and after" i18n-ariaLabel="@@analysis.compare.windowLength" />
      </span>
      <button class="btn" type="button" (click)="beforeAfter()" i18n="@@analysis.compare.beforeAfter">Before / after selected time</button>
      <button class="btn" type="button" [attr.aria-disabled]="!cursor.selection()" (click)="cursor.selection() && targets.setWindowA(cursor.selection())" i18n="@@analysis.compare.setA">Set A from selection</button>
      <button class="btn" type="button" [attr.aria-disabled]="!cursor.selection()" (click)="cursor.selection() && targets.setWindowB(cursor.selection())" i18n="@@analysis.compare.setB">Set B from selection</button>
    </div>
    <imt-coverage-note [coverage]="res.coverage()" />
    <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [retryable]="res.retryable()" [hasData]="!!res.result()" [empty]="!targets.windowA() || !targets.windowB()" emptyText="Pick two windows to compare, for example the current range against the one before it." i18n-emptyText="@@analysis.compare.emptyText" (retry)="res.retry()">
      @if (res.result(); as c) {
        <div class="body" tabindex="0" role="region" aria-label="Comparison" i18n-aria-label="@@analysis.compare.region">
          <p class="summary" data-testid="compare-summary">{{ summaryText(c) }}</p>

          <div class="cards">
            @for (side of sides(c); track side.name) {
              <section class="card" [attr.aria-label]="windowLabel(side.name)">
                <strong [class]="'side ' + side.name.toLowerCase()" i18n="@@analysis.compare.windowN.title">Window {{ side.name }}</strong>
                <div class="nums" i18n="@@analysis.compare.side.stats">{{ count(side.stats.count) }} events · {{ side.stats.perMinute }}/min · {{ duration(side.stats.durationMs) }}</div>
                <div class="lv">@for (l of side.stats.levels; track l.level) { <span class="lvl" [class]="'lvl ' + l.level">{{ l.level }} {{ count(l.count) }}</span> }</div>
              </section>
            }
          </div>

          <h3 class="sec" i18n="@@analysis.compare.changedFields">Changed fields</h3>
          @for (f of c.changedFields; track f.field) {
            <section class="field" [attr.aria-label]="fieldLabel(f.field)">
              <div class="fhead"><strong class="fname">{{ f.field }}</strong> <span class="muted nums" i18n="@@analysis.compare.distance">distance {{ dist(f.distance) }}</span></div>
              @for (v of shown(f); track v.value) {
                <div class="share">
                  <span class="val mono ellipsis" [title]="v.value">{{ v.value }}</span>
                  <span class="pair">
                    <span class="track"><span class="fill a" [style.width.%]="w(v.shareA)"></span></span>
                    <span class="track"><span class="fill b" [style.width.%]="w(v.shareB)"></span></span>
                  </span>
                  <span class="pcts nums"><span class="pa" i18n="@@analysis.compare.shareA">A {{ pct(v.shareA) }}</span><span class="pb" i18n="@@analysis.compare.shareB">B {{ pct(v.shareB) }}</span></span>
                </div>
              }
              @if (f.top.length > topLimit) {
                <button class="btn ghost small" type="button" (click)="toggleAll(f.field)">{{ expanded().has(f.field) ? hideLabel : showAllLabel }}</button>
              }
            </section>
          } @empty {
            <p class="muted" i18n="@@analysis.compare.noFieldChange">No field distribution changed noticeably.</p>
          }

          <h3 class="sec" i18n="@@analysis.compare.numericChanges">Numeric changes</h3>
          @if (c.numericChanges.length) {
            <table class="grid">
              <thead><tr><th scope="col" i18n="@@analysis.compare.col.field">Field</th><th scope="col" class="num" i18n="@@analysis.compare.col.medianA">Median A</th><th scope="col" class="num" i18n="@@analysis.compare.col.medianB">Median B</th><th scope="col" class="num" i18n="@@analysis.compare.col.change">Change</th><th scope="col" class="num" i18n="@@analysis.compare.col.p95A">p95 A</th><th scope="col" class="num" i18n="@@analysis.compare.col.p95B">p95 B</th><th scope="col" class="num" i18n="@@analysis.compare.col.change">Change</th></tr></thead>
              <tbody>
                @for (n of c.numericChanges; track n.field) {
                  <tr>
                    <td>{{ n.field }}</td>
                    <td class="num">{{ num(n.a.median) }}</td><td class="num">{{ num(n.b.median) }}</td>
                    <td [class]="'num ' + dir(n.medianChangePct)"><span aria-hidden="true">{{ arrow(n.medianChangePct) }}</span> {{ change(n.medianChangePct) }}</td>
                    <td class="num">{{ num(n.a.p95) }}</td><td class="num">{{ num(n.b.p95) }}</td>
                    <td [class]="'num ' + dir(n.p95ChangePct)"><span aria-hidden="true">{{ arrow(n.p95ChangePct) }}</span> {{ change(n.p95ChangePct) }}</td>
                  </tr>
                }
              </tbody>
            </table>
          } @else {
            <p class="muted" i18n="@@analysis.compare.noNumericChange">No numeric field changed noticeably.</p>
          }

          <div class="two">
            <div>
              <h3 class="sec" i18n="@@analysis.compare.newInB">New in B</h3>
              @for (a of c.newValues; track a.field) {
                <p><strong>{{ a.field }}</strong>: @for (v of a.values; track v.value) { <span class="chip">{{ v.value }} <span class="nums">{{ fmt(v.count) }}</span></span> }</p>
              } @empty { <p class="muted" i18n="@@analysis.compare.nothingNew">Nothing new.</p> }
            </div>
            <div>
              <h3 class="sec" i18n="@@analysis.compare.goneFromA">Gone from A</h3>
              @for (a of c.goneValues; track a.field) {
                <p><strong>{{ a.field }}</strong>: @for (v of a.values; track v.value) { <span class="chip">{{ v.value }} <span class="nums">{{ fmt(v.count) }}</span></span> }</p>
              } @empty { <p class="muted" i18n="@@analysis.compare.nothingGone">Nothing gone.</p> }
            </div>
          </div>
        </div>
      }
    </imt-analysis-state>
  `,
  styles: [
    ANALYSIS_STYLES,
    `
      .lbl { display: inline-flex; align-items: center; gap: 6px; color: var(--muted); }
      .nums { font-variant-numeric: tabular-nums; }
      .chip-ab { display: inline-flex; align-items: center; gap: 6px; font-weight: 600; color: var(--text); }
      .chip-ab .sw { inline-size: 10px; block-size: 10px; border-radius: 2px; }
      .chip-ab.a .sw, .side.a { background: var(--s1); }
      .chip-ab.b .sw, .side.b { background: var(--s2); }
      .side { background: none !important; }
      .side.a { color: var(--s1); }
      .side.b { color: var(--s2); }
      .btn[aria-disabled='true'] { cursor: default; }
      .summary { margin: 0 0 12px; color: var(--text); line-height: 1.5; max-width: 80ch; }
      .cards { grid-template-columns: 1fr 1fr; }
      .lv { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 6px; }
      .lvl { font-family: inherit; }
      .field { margin-bottom: 14px; }
      .fhead { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; max-width: 640px; margin-bottom: 4px; }
      .fname { font-size: var(--fs-md, 13px); font-weight: 600; color: var(--text); }
      .share { display: flex; gap: 10px; align-items: center; padding: 2px 0; max-width: 640px; }
      .val { width: 200px; flex: none; }
      .pair { display: flex; flex-direction: column; gap: 2px; flex: 1; }
      .track { height: 6px; background: var(--lane-bar); border-radius: 3px; overflow: hidden; }
      .fill { display: block; height: 100%; }
      .fill.a { background: var(--s1); }
      .fill.b { background: var(--s2); }
      .pcts { display: flex; gap: 12px; width: 150px; flex: none; font-size: var(--fs-xs); justify-content: flex-end; }
      .pcts span { width: 68px; text-align: right; white-space: nowrap; }
      .two { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
      .chip { margin-right: 4px; }
    `,
  ],
})
export class CompareView {
  protected readonly fmt = formatCount;
  protected fieldLabel(field: string): string {
    return $localize`:@@analysis.compare.fieldN:Field ${field}:field:`;
  }
  protected windowLabel(name: string): string {
    return $localize`:@@analysis.compare.windowN:Window ${name}:name:`;
  }
  protected readonly targets = inject(AnalysisTargets);
  protected readonly cursor = inject(TimeCursor);
  protected readonly time = inject(AnalysisTime);
  protected readonly res = new AnalysisResource<Comparison>();
  protected readonly widths = WIDTHS;
  protected readonly width = signal(WIDTHS[1]?.ms ?? 5 * 60_000);
  private readonly backend = inject(AnalysisBackend);

  protected readonly widthOptions: SelectOption<number>[] = WIDTHS.map((w) => ({ value: w.ms, label: w.label }));
  protected readonly topLimit = TOP_VALUES;
  protected readonly expanded = signal<ReadonlySet<string>>(new Set());
  protected readonly showAllLabel = $localize`:@@analysis.compare.showAll:Show all`;
  protected readonly hideLabel = $localize`:@@analysis.compare.showLess:Show top 5`;

  protected readonly count = formatCount;
  protected readonly duration = formatDuration;
  protected readonly num = formatNumber;
  protected readonly change = formatChange;

  constructor() {
    effect(() => {
      const a = this.targets.windowA();
      const b = this.targets.windowB();
      const base = this.targets.query();
      if (!a || !b) return untracked(() => this.res.reset());
      untracked(() => void this.res.load(() => this.backend.compareWindows(windowQuery(base, a), windowQuery(base, b))));
    });
  }

  protected sides(c: Comparison): { name: string; stats: SideStats }[] {
    return [
      { name: 'A', stats: c.a },
      { name: 'B', stats: c.b },
    ];
  }

  /** The first two or three sentences of the engine's summary, as one paragraph. */
  protected summaryText(c: Comparison): string {
    return c.summary.slice(0, 3).map((x) => (/[.!?]$/.test(x) ? x : `${x}.`)).join(' ');
  }

  protected dist(d: number): string {
    return (Math.round(d * 100) / 100).toString();
  }

  protected shown(f: FieldShift) {
    return this.expanded().has(f.field) ? f.top : f.top.slice(0, TOP_VALUES);
  }

  protected toggleAll(field: string): void {
    this.expanded.update((s) => {
      const n = new Set(s);
      if (!n.delete(field)) n.add(field);
      return n;
    });
  }

  protected w(share: number): number {
    return barWidth(share, 1);
  }

  protected pct(share: number): string {
    return `${Math.round(share * 1000) / 10}%`;
  }

  protected dir(pct: number | null): string {
    return direction(pct);
  }

  protected arrow(pct: number | null): string {
    return ARROWS[direction(pct)];
  }

  /** B = the current range (selection, query range or everything), A = the window just before it. */
  protected previous(): void {
    const r = this.targets.currentRange();
    if (r) this.targets.setWindows(previousWindow(r), r);
  }

  /** Around the selection's start, or the middle of the current range when nothing is selected. */
  protected beforeAfter(): void {
    const sel = this.cursor.selection();
    const range = this.targets.currentRange();
    const t = sel ? sel.from : range ? midpoint(range) : null;
    if (t === null) return;
    const w = aroundTime(t, this.width());
    this.targets.setWindows(w.a, w.b);
  }
}
