import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { RedactionSummary } from '../../core/api.types';
import { formatCount } from '../../core/format';

/** What redaction will change, before something is written: hits per rule and a few before/after samples. */
@Component({
  selector: 'imt-redaction-summary',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (summary(); as s) {
      <p class="line" role="status">
        <ng-container i18n="@@share.redactionSummary.changed"><strong>{{ fmt(s.changed) }}</strong> of {{ fmt(s.scanned) }}{{ s.truncated ? '+' : '' }} events change</ng-container>
        @if (hits().length) {
          ·
          @for (h of hits(); track h.rule; let last = $last) {
            <span class="mono">{{ h.rule }}</span> {{ fmt(h.count) }}{{ last ? '' : ', ' }}
          }
        }
      </p>
      @for (x of s.samples; track x.recordId) {
        <div class="sample">
          <div class="mono before" title="Before" i18n-title="@@share.redactionSummary.before">{{ x.before }}</div>
          <div class="mono after" title="After" i18n-title="@@share.redactionSummary.after">{{ x.after }}</div>
        </div>
      }
    } @else {
      <p class="line" i18n="@@share.redactionSummary.checking">Checking what will be redacted…</p>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-sm); }
    .line { margin: 0; color: var(--text-2); }
    .sample { display: grid; gap: 2px; padding: 6px 8px; border: 1px solid var(--border); border-radius: 6px; background: var(--code-bg); }
    .before, .after { font-size: var(--fs-xs); white-space: pre-wrap; word-break: break-word; max-height: 3.2em; overflow: hidden; }
    .before { color: var(--muted); text-decoration: line-through; }
    .after { color: var(--text); }
  `,
})
export class RedactionSummaryView {
  readonly summary = input<RedactionSummary | null>(null);
  protected readonly hits = computed(() => (this.summary()?.hits ?? []).filter((h) => h.count > 0));
  protected readonly fmt = formatCount;
}
