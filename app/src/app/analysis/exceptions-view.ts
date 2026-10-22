import { ChangeDetectionStrategy, Component, effect, ElementRef, inject, signal, untracked } from '@angular/core';
import { formatCount } from '../core/format';
import { ExceptionGroup } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { ExceptionFocus } from './exception-focus';
import { AnalysisActions } from './analysis-actions';
import { AnalysisState } from './analysis-state';
import { ANALYSIS_STYLES } from './analysis-styles';
import { AnalysisTargets } from './analysis-targets';
import { AnalysisTime } from './analysis-time';
import { AnalysisResource } from './async-resource';
import { CoverageNote } from './coverage-note';

/** Exceptions grouped by fingerprint; a group expands into its instances and sample events. */
@Component({
  selector: 'debrief-exceptions-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, CoverageNote],
  template: `
    <div class="bar">
      <span class="section-title" i18n="@@analysis.exceptions.title">Exceptions</span>
      @if (res.result(); as list) { <span class="faint" i18n="@@analysis.exceptions.groups">{{ list.length }} groups</span> }
      <span class="spacer"></span>
      <button class="btn" type="button" (click)="res.retry()" i18n="@@analysis.exceptions.refresh">Refresh</button>
    </div>
    <debrief-coverage-note [coverage]="res.coverage()" />
    <debrief-analysis-state [loading]="res.loading()" [error]="res.error()" [retryable]="res.retryable()" [hasData]="!!res.result()" [empty]="res.result()?.length === 0" emptyText="No exceptions in the current query." i18n-emptyText="@@analysis.exceptions.emptyText" emptyHint="Events that carry an exception (a stack trace) are grouped here by type and top frame." i18n-emptyHint="@@analysis.exceptions.emptyHint" suggest (retry)="res.retry()">
      <ul class="list body" aria-label="Exception groups" i18n-aria-label="@@analysis.exceptions.listLabel">
        @for (g of res.result() ?? []; track g.fingerprint) {
          <li class="group" [class.selected]="selected() === g.fingerprint" [attr.data-fingerprint]="g.fingerprint">
            <button class="row head" type="button" [attr.aria-expanded]="isOpen(g)" [attr.aria-current]="selected() === g.fingerprint ? 'true' : null" (click)="toggle(g)">
              <span aria-hidden="true">{{ isOpen(g) ? '▾' : '▸' }}</span>
              <span class="num count">{{ count(g.count) }}</span>
              <span class="type mono">{{ g.typeName }}</span>
              <span class="grow ellipsis" [title]="g.message">{{ g.message }}</span>
              <span class="mono muted frame ellipsis" [title]="g.topFrame ?? ''">{{ g.topFrame ?? '' }}</span>
            </button>
            @if (isOpen(g)) {
              <div class="detail">
                <h3 class="sec" i18n="@@analysis.exceptions.instances">Instances</h3>
                <ul class="list">
                  @for (i of g.instances; track i[0]) {
                    <li class="inst"><span class="ellipsis">{{ i[0] }}</span><span class="num">{{ count(i[1]) }}</span></li>
                  }
                </ul>
                <p class="muted">{{ time.format(g.firstTs) }} → {{ time.format(g.lastTs) }}</p>
                <h3 class="sec" i18n="@@analysis.exceptions.sampleEvents">Sample events</h3>
                <div class="samples">
                  @for (id of g.sampleIds; track id) {
                    <button class="btn" type="button" (click)="select(id)" i18n="@@analysis.exceptions.event">Event {{ id }}</button>
                  }
                </div>
              </div>
            }
          </li>
        }
      </ul>
    </debrief-analysis-state>
  `,
  styles: [
    ANALYSIS_STYLES,
    `
      .list.body { padding: 0; }
      .count { width: 48px; flex: none; color: var(--text); }
      .type { flex: none; color: var(--danger-text); max-width: 280px; overflow: hidden; text-overflow: ellipsis; }
      .frame { max-width: 320px; flex: none; font-size: var(--fs-xs); }
      .group { border-bottom: 1px solid var(--border); }
      .head { border-bottom: 0; }
      .group.selected { background: var(--bg-selected, var(--surface-2)); box-shadow: inset 2px 0 0 var(--accent); }
      .detail { padding: 0 12px 10px 38px; }
      .inst { display: flex; gap: 12px; padding: 2px 0; max-width: 520px; }
      .inst .ellipsis { flex: 1; }
      .samples { display: flex; flex-wrap: wrap; gap: 6px; }
    `,
  ],
})
export class ExceptionsView {
  protected readonly res = new AnalysisResource<ExceptionGroup[]>();
  protected readonly time = inject(AnalysisTime);
  protected readonly count = formatCount;
  protected readonly open = signal<ReadonlySet<string>>(new Set());
  private readonly backend = inject(AnalysisBackend);
  protected readonly targets = inject(AnalysisTargets);
  private readonly actions = inject(AnalysisActions);
  private readonly focus = inject(ExceptionFocus);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);
  /** The group chosen from elsewhere (the search palette): expanded, highlighted and scrolled into view. */
  protected readonly selected = signal<string | null>(null);

  constructor() {
    effect(() => {
      const fingerprint = this.focus.fingerprint();
      const groups = this.res.result();
      if (fingerprint === null || !groups) return;
      untracked(() => {
        this.focus.clear();
        if (!groups.some((g) => g.fingerprint === fingerprint)) return;
        this.selected.set(fingerprint);
        this.open.set(new Set(this.open()).add(fingerprint));
        setTimeout(() => this.host.nativeElement.querySelector(`[data-fingerprint="${CSS.escape(fingerprint)}"]`)?.scrollIntoView?.({ block: 'nearest' }));
      });
    });
    effect(() => {
      const query = this.targets.query();
      untracked(() => void this.res.load(() => this.backend.queryExceptions(query)));
    });
  }

  protected isOpen(g: ExceptionGroup): boolean {
    return this.open().has(g.fingerprint);
  }

  protected toggle(g: ExceptionGroup): void {
    const next = new Set(this.open());
    if (!next.delete(g.fingerprint)) next.add(g.fingerprint);
    this.open.set(next);
  }

  protected select(id: number): void {
    void this.actions.jumpTo(id);
  }
}
