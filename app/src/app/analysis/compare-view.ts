import { ChangeDetectionStrategy, Component, effect, inject, signal, untracked } from '@angular/core';
import { formatCount, formatDuration, formatNumber } from '../core/format';
import { Comparison, SideStats } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { barWidth, direction, ARROWS, formatChange } from './analysis-format';
import { AnalysisState } from './analysis-state';
import { ANALYSIS_STYLES } from './analysis-styles';
import { AnalysisTargets, windowQuery } from './analysis-targets';
import { AnalysisTime } from './analysis-time';
import { AsyncResource } from './async-resource';
import { TimeCursor } from './time-cursor';
import { aroundTime, midpoint, previousWindow, WIDTHS } from './windows';

/** Two time windows side by side: counts, which fields changed, numeric shifts, new and gone values. */
@Component({
  selector: 'imt-compare-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState],
  template: `
    <div class="bar">
      <span class="section-title">Compare</span>
      <button class="btn" type="button" (click)="previous()">Current range vs previous</button>
      <label class="lbl">Window
        <select class="select" [value]="width()" (change)="width.set(+$any($event.target).value)" aria-label="Window length for before and after">
          @for (w of widths; track w.ms) { <option [value]="w.ms" [selected]="w.ms === width()">{{ w.label }}</option> }
        </select>
      </label>
      <button class="btn" type="button" (click)="beforeAfter()">Before / after selected time</button>
      <button class="btn" type="button" [disabled]="!cursor.selection()" (click)="targets.setWindowA(cursor.selection())">Set A from selection</button>
      <button class="btn" type="button" [disabled]="!cursor.selection()" (click)="targets.setWindowB(cursor.selection())">Set B from selection</button>
    </div>
    <div class="windows">
      <div><strong>A</strong> <span class="mono">{{ time.range(targets.windowA()) }}</span></div>
      <div><strong>B</strong> <span class="mono">{{ time.range(targets.windowB()) }}</span></div>
    </div>
    <imt-analysis-state [loading]="res.loading()" [error]="res.error()" [hasData]="!!res.data()" [empty]="!targets.windowA() || !targets.windowB()" emptyText="Pick two windows to compare, for example the current range against the one before it." (retry)="res.retry()">
      @if (res.data(); as c) {
        <div class="body" tabindex="0" role="region" aria-label="Comparison">
          <h3 class="sec first">Summary</h3>
          <ul class="summary">
            @for (s of c.summary; track $index) { <li>{{ s }}</li> }
          </ul>

          <div class="cards">
            @for (side of sides(c); track side.name) {
              <section class="card" [attr.aria-label]="'Window ' + side.name">
                <strong>Window {{ side.name }}</strong>
                <div class="mono">{{ count(side.stats.count) }} events · {{ side.stats.perMinute }}/min · {{ duration(side.stats.durationMs) }}</div>
                <div class="lv">@for (l of side.stats.levels; track l.level) { <span class="lvl" [class]="'lvl ' + l.level">{{ l.level }} {{ count(l.count) }}</span> }</div>
              </section>
            }
          </div>

          <h3 class="sec">Changed fields</h3>
          @for (f of c.changedFields; track f.field) {
            <section class="field" [attr.aria-label]="'Field ' + f.field">
              <div><strong class="mono">{{ f.field }}</strong> <span class="muted">distance {{ f.distance }}</span></div>
              @for (v of f.top; track v.value) {
                <div class="share">
                  <span class="val mono ellipsis" [title]="v.value">{{ v.value }}</span>
                  <span class="pair">
                    <span class="track"><span class="fill a" [style.width.%]="w(v.shareA)"></span></span>
                    <span class="track"><span class="fill b" [style.width.%]="w(v.shareB)"></span></span>
                  </span>
                  <span class="mono nums">A {{ pct(v.shareA) }} · B {{ pct(v.shareB) }}</span>
                </div>
              }
            </section>
          } @empty {
            <p class="muted">No field distribution changed noticeably.</p>
          }

          <h3 class="sec">Numeric changes</h3>
          @if (c.numericChanges.length) {
            <table class="grid">
              <thead><tr><th scope="col">Field</th><th scope="col" class="num">Median A</th><th scope="col" class="num">Median B</th><th scope="col" class="num">Change</th><th scope="col" class="num">p95 A</th><th scope="col" class="num">p95 B</th><th scope="col" class="num">Change</th></tr></thead>
              <tbody>
                @for (n of c.numericChanges; track n.field) {
                  <tr>
                    <td class="mono">{{ n.field }}</td>
                    <td class="num">{{ num(n.a.median) }}</td><td class="num">{{ num(n.b.median) }}</td>
                    <td [class]="'num ' + dir(n.medianChangePct)"><span aria-hidden="true">{{ arrow(n.medianChangePct) }}</span> {{ change(n.medianChangePct) }}</td>
                    <td class="num">{{ num(n.a.p95) }}</td><td class="num">{{ num(n.b.p95) }}</td>
                    <td [class]="'num ' + dir(n.p95ChangePct)"><span aria-hidden="true">{{ arrow(n.p95ChangePct) }}</span> {{ change(n.p95ChangePct) }}</td>
                  </tr>
                }
              </tbody>
            </table>
          } @else {
            <p class="muted">No numeric field changed noticeably.</p>
          }

          <div class="two">
            <div>
              <h3 class="sec">New in B</h3>
              @for (a of c.newValues; track a.field) {
                <p><strong class="mono">{{ a.field }}</strong>: @for (v of a.values; track v.value) { <span class="chip">{{ v.value }} <span class="mono">{{ v.count }}</span></span> }</p>
              } @empty { <p class="muted">Nothing new.</p> }
            </div>
            <div>
              <h3 class="sec">Gone from A</h3>
              @for (a of c.goneValues; track a.field) {
                <p><strong class="mono">{{ a.field }}</strong>: @for (v of a.values; track v.value) { <span class="chip">{{ v.value }} <span class="mono">{{ v.count }}</span></span> }</p>
              } @empty { <p class="muted">Nothing gone.</p> }
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
      .windows { flex: none; display: flex; gap: 24px; padding: 6px 12px; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
      .sec.first { margin-top: 4px; }
      .summary { margin: 0; padding-left: 18px; color: var(--text); }
      .lv { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 6px; }
      .field { margin-bottom: 10px; }
      .share { display: flex; gap: 10px; align-items: center; padding: 2px 0; }
      .val { width: 200px; flex: none; }
      .pair { display: flex; flex-direction: column; gap: 2px; flex: 1; max-width: 360px; }
      .track { height: 6px; background: var(--lane-bar); border-radius: 3px; overflow: hidden; }
      .fill { display: block; height: 100%; }
      .fill.a { background: var(--s1); }
      .fill.b { background: var(--s2); }
      .nums { width: 160px; flex: none; font-size: 11px; }
      .two { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
      .chip { margin-right: 4px; }
    `,
  ],
})
export class CompareView {
  protected readonly targets = inject(AnalysisTargets);
  protected readonly cursor = inject(TimeCursor);
  protected readonly time = inject(AnalysisTime);
  protected readonly res = new AsyncResource<Comparison>();
  protected readonly widths = WIDTHS;
  protected readonly width = signal(WIDTHS[1].ms);
  private readonly backend = inject(AnalysisBackend);

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
