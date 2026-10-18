import { inject, Injectable } from '@angular/core';
import { CommandContext, ImtCommand } from '../extensions/tokens';
import { AnalysisTargets } from './analysis-targets';
import { QuerySession } from './query-session';
import { TimeCursor } from './time-cursor';
import { aroundTime, midpoint, previousWindow, WIDTHS } from './windows';

const loaded = (ctx: CommandContext) => !!ctx.summary;
const CATEGORY = $localize`:@@analysis.commands.category:Analyze`;

/** Palette commands of the analysis features. */
@Injectable({ providedIn: 'root' })
export class AnalysisCommands {
  private readonly targets = inject(AnalysisTargets);
  private readonly cursor = inject(TimeCursor);
  private readonly session = inject(QuerySession);

  private show(id: string, title: string, viewId: string): ImtCommand {
    return { id, title, category: CATEGORY, when: loaded, run: (ctx) => ctx.showView(viewId) };
  }

  readonly commands: ImtCommand[] = [
    { id: 'query.focus', title: $localize`:@@analysis.commands.queryFocus:Query: focus query bar`, category: CATEGORY, keybinding: 'Mod+/', when: loaded, run: (ctx) => {
      ctx.showView('query');
      this.session.requestFocus();
    } },
    { id: 'query.run', title: $localize`:@@analysis.commands.queryRun:Query: run`, category: CATEGORY, when: loaded, run: async () => void (await this.session.run()) },
    { id: 'query.clear', title: $localize`:@@analysis.commands.queryClear:Query: clear applied query`, category: CATEGORY, when: loaded, run: () => this.session.clearApplied() },
    this.show('query.results', $localize`:@@analysis.commands.queryResults:Query: show results`, 'query-results'),
    {
      id: 'moment.analyse',
      title: $localize`:@@analysis.commands.momentAnalyse:Analyse this moment`,
      category: CATEGORY,
      keybinding: 'Mod+Shift+M',
      when: loaded,
      run: (ctx) => {
        this.targets.setMoment(this.targets.currentRange());
        ctx.showView('moment');
      },
    },
    {
      id: 'moment.selected',
      title: $localize`:@@analysis.commands.momentSelected:Moment: selected range`,
      category: CATEGORY,
      when: () => !!this.cursor.selection(),
      run: (ctx) => {
        this.targets.setMoment(this.cursor.selection());
        ctx.showView('moment');
      },
    },
    this.show('anomalies.show', $localize`:@@analysis.commands.anomalies:Show anomalies`, 'anomalies'),
    this.show('patterns.show', $localize`:@@analysis.commands.patterns:Show patterns`, 'patterns'),
    this.show('exceptions.show', $localize`:@@analysis.commands.exceptions:Show exceptions`, 'exceptions'),
    this.show('compare.show', $localize`:@@analysis.commands.compare:Show compare`, 'compare'),
    {
      id: 'compare.previous',
      title: $localize`:@@analysis.commands.comparePrevious:Compare: current range vs previous`,
      category: CATEGORY,
      when: loaded,
      run: (ctx) => {
        const r = this.targets.currentRange();
        if (r) this.targets.setWindows(previousWindow(r), r);
        ctx.showView('compare');
      },
    },
    {
      id: 'compare.beforeAfter',
      title: $localize`:@@analysis.commands.compareBeforeAfter:Compare: before / after selected time`,
      category: CATEGORY,
      when: loaded,
      run: (ctx) => {
        const sel = this.cursor.selection();
        const range = this.targets.currentRange();
        const t = sel ? sel.from : range ? midpoint(range) : null;
        if (t !== null) {
          const w = aroundTime(t, WIDTHS[1]?.ms ?? 5 * 60_000);
          this.targets.setWindows(w.a, w.b);
        }
        ctx.showView('compare');
      },
    },
    {
      id: 'fieldstats.show',
      title: $localize`:@@analysis.commands.fieldStats:Field statistics…`,
      category: CATEGORY,
      when: loaded,
      run: (ctx) => ctx.showView('field-stats'),
    },
    this.show('traces.show', $localize`:@@analysis.commands.traces:Show traces and requests`, 'traces'),
  ];
}
