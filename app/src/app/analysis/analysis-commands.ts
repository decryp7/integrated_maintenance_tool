import { inject, Injectable } from '@angular/core';
import { CommandContext, ImtCommand } from '../extensions/tokens';
import { AnalysisTargets } from './analysis-targets';
import { QuerySession } from './query-session';
import { TimeCursor } from './time-cursor';
import { aroundTime, midpoint, previousWindow, WIDTHS } from './windows';

const loaded = (ctx: CommandContext) => !!ctx.summary;
const CATEGORY = 'Analyze';

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
    { id: 'query.focus', title: 'Query: focus query bar', category: CATEGORY, keybinding: 'Mod+/', when: loaded, run: (ctx) => {
      ctx.showView('query');
      this.session.requestFocus();
    } },
    { id: 'query.run', title: 'Query: run', category: CATEGORY, when: loaded, run: async () => void (await this.session.run()) },
    { id: 'query.clear', title: 'Query: clear applied query', category: CATEGORY, when: loaded, run: () => this.session.clearApplied() },
    this.show('query.results', 'Query: show results', 'query-results'),
    {
      id: 'moment.analyse',
      title: 'Analyse this moment',
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
      title: 'Moment: selected range',
      category: CATEGORY,
      when: () => !!this.cursor.selection(),
      run: (ctx) => {
        this.targets.setMoment(this.cursor.selection());
        ctx.showView('moment');
      },
    },
    this.show('anomalies.show', 'Show anomalies', 'anomalies'),
    this.show('patterns.show', 'Show patterns', 'patterns'),
    this.show('exceptions.show', 'Show exceptions', 'exceptions'),
    this.show('compare.show', 'Show compare', 'compare'),
    {
      id: 'compare.previous',
      title: 'Compare: current range vs previous',
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
      title: 'Compare: before / after selected time',
      category: CATEGORY,
      when: loaded,
      run: (ctx) => {
        const sel = this.cursor.selection();
        const range = this.targets.currentRange();
        const t = sel ? sel.from : range ? midpoint(range) : null;
        if (t !== null) {
          const w = aroundTime(t, WIDTHS[1].ms);
          this.targets.setWindows(w.a, w.b);
        }
        ctx.showView('compare');
      },
    },
    {
      id: 'fieldstats.show',
      title: 'Field statistics…',
      category: CATEGORY,
      when: loaded,
      run: (ctx) => ctx.showView('field-stats'),
    },
    this.show('traces.show', 'Show traces and requests', 'traces'),
  ];
}
