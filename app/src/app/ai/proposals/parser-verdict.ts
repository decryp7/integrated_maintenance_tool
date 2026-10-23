import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';
import { FailedRule, ParserProposal, ZoneSource } from '../ai.types';
import { formatCount } from '../../shared/format/units';

const RULES: Record<FailedRule, string> = {
  format: $localize`:@@ai.parserVerdict.rule.format:no pattern`,
  missedEntry: $localize`:@@ai.parserVerdict.rule.missedEntry:missed entry`,
  timestamp: $localize`:@@ai.parserVerdict.rule.timestamp:timestamp`,
  level: $localize`:@@ai.parserVerdict.rule.level:level`,
};

const ZONES: Record<ZoneSource, string> = {
  timestamps: $localize`:@@ai.parserVerdict.zone.timestamps:offsets in the timestamps`,
  'same-machine': $localize`:@@ai.parserVerdict.zone.machine:other files of this machine`,
  'modification-time': $localize`:@@ai.parserVerdict.zone.mtime:the file's modification time`,
  'text-hint': $localize`:@@ai.parserVerdict.zone.text:a zone named in the file`,
  asked: $localize`:@@ai.parserVerdict.zone.asked:not decided: please check`,
  user: $localize`:@@ai.parserVerdict.zone.user:set by you`,
};

/**
 * The engine's verdict on a generated parser (ADR 0032): the score against the target, the lines it still cannot
 * read (line, rule, fragment, reason), the time zone and where it came from, the identity fields and metrics it
 * kept or dropped, and the rounds of drafting and checking. Display only: every number comes from Rust.
 */
@Component({
  selector: 'debrief-ai-parser-verdict',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (p().verdict; as v) {
      <div class="verdict" data-testid="parser-verdict">
        <p class="score" [class.ok]="p().status !== 'bestEffort'">
          <strong data-testid="parser-score">{{ percent(v.score) }}</strong>
          <span i18n="@@ai.parserVerdict.score">of {{ count(v.lines) }} lines read with a timestamp{{ v.readsLevels ? levelSuffix : '' }}</span>
          <span class="status" data-testid="parser-status">{{ statusText() }}</span>
        </p>
        @if (!v.timestamps.accepted && v.timestamps.reason) {
          <p class="warn" i18n="@@ai.parserVerdict.timestamps">Timestamps: {{ v.timestamps.reason }}</p>
        }
        @if (v.failures.length) {
          <details [open]="p().status === 'bestEffort'">
            <summary i18n="@@ai.parserVerdict.unreadable">{{ count(v.lines - v.readable) }} lines not read — examples</summary>
            <table class="failures">
              <thead><tr><th scope="col" i18n="@@ai.parserVerdict.col.line">Line</th><th scope="col" i18n="@@ai.parserVerdict.col.rule">Rule</th><th scope="col" i18n="@@ai.parserVerdict.col.text">Text</th></tr></thead>
              <tbody>
                @for (f of v.failures; track $index) {
                  <tr [attr.title]="f.reason"><td>{{ f.line }}</td><td>{{ rule(f.rule) }}</td><td class="mono">{{ f.fragment }}</td></tr>
                }
              </tbody>
            </table>
          </details>
        }
        <p class="facts" data-testid="parser-zone">
          <ng-container i18n="@@ai.parserVerdict.zoneLine">Time zone {{ v.zone.offset }} from {{ zone(v.zone.source) }}</ng-container>
          @if (v.zone.confidence > 0 && v.zone.source !== 'timestamps') {
            <span class="muted"> ({{ percent(v.zone.confidence) }})</span>
          }
          <span class="muted"> — {{ v.zone.detail }}</span>
        </p>
        @if (v.identity.length) {
          <ul class="checks" aria-label="Identity fields" i18n-aria-label="@@ai.parserVerdict.identity">
            @for (c of v.identity; track c.field) {
              <li [class.dropped]="!c.accepted"><span class="mono">{{ c.field }}</span> {{ c.accepted ? kept : dropped }} · {{ c.reason }}</li>
            }
          </ul>
        }
        @if (v.metrics.length) {
          <ul class="checks" aria-label="Metrics" i18n-aria-label="@@ai.parserVerdict.metrics">
            @for (m of v.metrics; track $index) {
              <li [class.dropped]="!m.accepted"><span class="mono">{{ m.name }}</span> {{ m.accepted ? kept : dropped }} · {{ m.reason }}</li>
            }
          </ul>
        }
        @if (p().chartsToml) {
          <details><summary i18n="@@ai.parserVerdict.charts">Charts for the profile (one line per instance)</summary><pre class="mono">{{ p().chartsToml }}</pre></details>
        }
        @if (p().rounds.length) {
          <ol class="rounds" aria-label="Rounds" i18n-aria-label="@@ai.parserVerdict.rounds">
            @for (r of p().rounds; track r.round) {
              <li><ng-container i18n="@@ai.parserVerdict.round">Round {{ r.round }}: {{ percent(r.score) }}</ng-container>@if (r.note) {<span class="muted"> · {{ r.note }}</span>}</li>
            }
          </ol>
        }
      </div>
    }
  `,
  styles: `
    .verdict { display: grid; gap: 6px; margin: 6px 0; }
    .score { margin: 0; display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
    .score strong { font-size: 1.2em; color: var(--warn-text); }
    .score.ok strong { color: var(--ok); }
    .status { margin-left: auto; color: var(--muted); }
    .warn { margin: 0; color: var(--warn-text); }
    .facts { margin: 0; }
    .muted { color: var(--muted); }
    .mono { font-family: var(--font-mono); }
    table.failures { width: 100%; border-collapse: collapse; font-size: 0.9em; }
    table.failures td, table.failures th { text-align: left; padding: 2px 6px; vertical-align: top; }
    table.failures td.mono { word-break: break-all; }
    .checks, .rounds { margin: 0; padding-left: 18px; }
    .dropped { color: var(--muted); text-decoration: line-through; text-decoration-thickness: 1px; }
  `,
})
export class ParserVerdictView {
  readonly p = input.required<ParserProposal>();
  protected readonly count = formatCount;

  protected readonly levelSuffix = $localize`:@@ai.parserVerdict.andLevel: and a level`;
  protected readonly kept = $localize`:@@ai.parserVerdict.kept:kept`;
  protected readonly dropped = $localize`:@@ai.parserVerdict.dropped:dropped`;
  protected readonly statusText = computed(() => {
    switch (this.p().status) {
      case 'done':
        return $localize`:@@ai.parserVerdict.status.done:target reached`;
      case 'builtin':
        return $localize`:@@ai.parserVerdict.status.builtin:read by Debrief's parsers`;
      default:
        return $localize`:@@ai.parserVerdict.status.best:best after ${this.p().rounds.length}:rounds: rounds — edit to finish`;
    }
  });

  protected percent(x: number): string {
    return `${Math.floor(x * 1000) / 10}%`;
  }

  protected rule(r: FailedRule): string {
    return RULES[r] ?? r;
  }

  protected zone(z: ZoneSource): string {
    return ZONES[z] ?? z;
  }
}
