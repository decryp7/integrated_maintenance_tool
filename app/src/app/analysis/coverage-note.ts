import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { Coverage, isPartial } from '../core/api.types';
import { formatCount } from '../core/format';

/**
 * Says when an analysis covers only part of the selection: the engine reports with each result how many of the
 * matching events it looked at and which limits bounded it (`Coverage`). Shows nothing when everything was
 * analysed or there is no result yet.
 */
@Component({
  selector: 'debrief-coverage-note',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (partial(); as c) {
      <div class="note" role="note">
        @if (c.analysed < c.matched) {
          <p i18n="@@analysis.coverage.partial">Analysed the first {{ count(c.analysed) }} of {{ count(c.matched) }} matching events. Narrow the query to analyse all of them.</p>
        } @else {
          <p i18n="@@analysis.coverage.limited">The result was limited:</p>
        }
        @if (c.limits.length) {
          <ul aria-label="Limits" i18n-aria-label="@@analysis.coverage.limits">
            @for (l of c.limits; track $index) { <li>{{ l }}</li> }
          </ul>
        }
      </div>
    }
  `,
  styles: [
    `
      .note { margin: 0; padding: 4px 12px; font-size: var(--fs-sm); color: var(--warn-text); background: color-mix(in srgb, var(--warn-text) 8%, transparent); border-bottom: 1px solid var(--border); }
      p, ul { margin: 0; }
      ul { padding-left: 18px; }
    `,
  ],
})
export class CoverageNote {
  /** The coverage of the analysis reply shown next to the note; null while there is none. */
  readonly coverage = input<Coverage | null>(null);
  protected readonly partial = computed(() => {
    const c = this.coverage();
    return c && isPartial(c) ? c : null;
  });
  protected readonly count = formatCount;
}
