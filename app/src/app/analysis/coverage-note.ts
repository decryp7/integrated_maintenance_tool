import { ChangeDetectionStrategy, Component, effect, inject, input, signal, untracked } from '@angular/core';
import { AnalysisCoverage, QlQuery } from '../core/api.types';
import { AnalysisBackend } from '../core/analysis-backend';
import { formatCount } from '../core/format';

/**
 * Says when an analysis covers only part of the selection: the engine looks at a bounded number of the
 * matching events (the first in time order) and reports how many of how many (`analysis_coverage`).
 * Shows nothing when everything was analysed or the engine cannot say.
 */
@Component({
  selector: 'imt-coverage-note',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (partial(); as c) {
      <p class="note" role="note" i18n="@@analysis.coverage.partial">Analysed the first {{ count(c.analysed) }} of {{ count(c.total) }} matching events. Narrow the query to analyse all of them.</p>
    }
  `,
  styles: [`.note { margin: 0; padding: 4px 12px; font-size: var(--fs-sm); color: var(--warn-text); background: color-mix(in srgb, var(--warn-text) 8%, transparent); border-bottom: 1px solid var(--border); }`],
})
export class CoverageNote {
  readonly query = input.required<QlQuery>();
  protected readonly partial = signal<AnalysisCoverage | null>(null);
  protected readonly count = formatCount;
  private readonly backend = inject(AnalysisBackend);
  private asked = 0;

  constructor() {
    effect(() => {
      const query = this.query();
      untracked(() => void this.load(query));
    });
  }

  private async load(query: QlQuery): Promise<void> {
    const ask = ++this.asked;
    let coverage: AnalysisCoverage | null;
    try {
      coverage = await this.backend.analysisCoverage(query);
    } catch {
      coverage = null; // The note is a hint: without an answer it stays hidden.
    }
    if (ask !== this.asked) return;
    this.partial.set(coverage && coverage.analysed < coverage.total ? coverage : null);
  }
}
