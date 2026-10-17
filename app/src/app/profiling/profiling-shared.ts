import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { Measurement, MetricId, RecursionMode } from '../core/api.types';
import { CapturesStore } from './captures.store';
import { formatNs, measurementGlyph, measurementHelp, measurementLabel } from './profiling-format';
import { PROFILING_STYLES } from './profiling-styles';
import { ActiveProfile } from './profiling-navigator';
import { cleanPatterns, ProfileViewState } from './profile-view-state';

let tagIds = 0;

/**
 * The "sampled (≈) / exact (=)" tag of a value or column. The glyph is text (never colour alone),
 * the meaning is in the tooltip and spelled out for screen readers.
 */
@Component({
  selector: 'imt-measurement',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<span class="m" [class.sampled]="m() === 'sampled'" [attr.title]="help()" [attr.data-measurement]="m()" aria-hidden="true">{{ glyph() }}</span><span class="sr">({{ label() }}{{ m() === 'sampled' ? ', estimate' : '' }})</span>`,
  styles: `
    :host { display: inline-flex; }
    .m { font-family: var(--font-mono); font-size: 10px; min-width: 12px; text-align: center; padding: 0 3px; border-radius: 3px; border: 1px solid var(--border-3); color: var(--text-2); cursor: help; }
    .m.sampled { border-style: dashed; }
    .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  `,
})
export class MeasurementTag {
  readonly m = input.required<Measurement>();
  protected readonly glyph = computed(() => measurementGlyph(this.m()));
  protected readonly label = computed(() => measurementLabel(this.m()));
  protected readonly help = computed(() => measurementHelp(this.m()));
}

/** Explains the measurement tags once per view. */
@Component({
  selector: 'imt-measurement-legend',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MeasurementTag],
  template: `<span class="lg"><imt-measurement m="sampled" /> sampled estimate</span><span class="lg"><imt-measurement m="exact" /> exact</span>`,
  styles: `:host { display: inline-flex; gap: 10px; font-size: 11px; color: var(--muted); } .lg { display: inline-flex; gap: 4px; align-items: center; white-space: nowrap; }`,
})
export class MeasurementLegend {}

/** Import notes (truncations, limits, unresolved frames) shown as a banner. */
@Component({
  selector: 'imt-capture-notes',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (notes().length) {
      <div class="notes" role="note" [attr.aria-label]="'Import notes for ' + name()">
        <strong>Import notes</strong>
        <ul>
          @for (n of notes(); track $index) { <li>{{ n }}</li> }
        </ul>
      </div>
    }
  `,
  styles: `
    .notes { display: flex; gap: 10px; align-items: baseline; padding: 4px 12px; font-size: 12px; border-bottom: 1px solid var(--border); background: var(--bg-2); color: var(--warn-text); }
    .notes strong { white-space: nowrap; }
    ul { margin: 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 2px; color: var(--text-2); }
  `,
})
export class CaptureNotes {
  readonly notes = input<string[]>([]);
  readonly name = input('');
}

/** "Open a profile" empty state with the open action. */
@Component({
  selector: 'imt-no-capture',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="state" role="status">
      <span>{{ text() }}</span>
      <button class="btn" type="button" [disabled]="captures.loading()" (click)="captures.pickAndOpen()">Open profile…</button>
      @if (captures.error(); as e) { <span class="err" role="alert">{{ e }}</span> }
    </div>
  `,
  styles: `
    .state { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; padding: 28px 16px; color: var(--muted); text-align: center; }
    .err { color: var(--danger-text); white-space: pre-line; }
  `,
})
export class NoCapture {
  readonly text = input('Open a profile to see this view.');
  protected readonly captures = inject(CapturesStore);
}

const RECURSION: { id: RecursionMode; label: string }[] = [
  { id: 'off', label: 'No folding' },
  { id: 'direct', label: 'Fold direct recursion' },
  { id: 'all', label: 'Fold all recursion' },
];

/**
 * The shared profile filters above every profile view: capture, metric, range and threads,
 * just-my-code (with its framework patterns) and recursion folding. Changing one re-queries all views.
 */
@Component({
  selector: 'imt-profile-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MeasurementTag],
  template: `
    @if (active.profile(); as p) {
      <div class="bar" role="toolbar" aria-label="Profile view">
        <label class="inline">Profile
          <select [value]="p.id" (change)="pickProfile($event)" aria-label="Active profile">
            @for (c of captures.profiles(); track c.id) { <option [value]="c.id" [selected]="c.id === p.id">{{ c.name }}</option> }
          </select>
        </label>
        @if (showMetric()) {
          <label class="inline">Metric
            <select (change)="pickMetric($event)" aria-label="Metric">
              @for (m of p.metrics; track m.id) { <option [value]="m.id" [selected]="m.id === active.metric()">{{ m.label }} ({{ m.measurement }})</option> }
            </select>
          </label>
          @if (measurement(); as m) { <imt-measurement [m]="m" /> }
        }
        @if (state.range(); as r) {
          <span class="pill" data-testid="range-pill">Range {{ ns(r.fromNs) }} – {{ ns(r.toNs) }}<button type="button" aria-label="Clear time range" (click)="state.setRange(null)">✕</button></span>
        }
        @if (state.threads(); as t) {
          <span class="pill">{{ t.length }} thread{{ t.length === 1 ? '' : 's' }}<button type="button" aria-label="Show all threads" (click)="state.setThreads(null)">✕</button></span>
        }
        <label class="inline"><input type="checkbox" [checked]="state.justMyCode()" (change)="state.setJustMyCode($any($event.target).checked)" /> Just my code</label>
        <button class="mini" type="button" [attr.aria-expanded]="editing()" [attr.aria-controls]="editorId" (click)="editing.set(!editing())">Framework patterns…</button>
        <label class="inline">Recursion
          <select (change)="state.setRecursion($any($event.target).value)" aria-label="Recursion folding">
            @for (r of recursion; track r.id) { <option [value]="r.id" [selected]="r.id === state.recursion()">{{ r.label }}</option> }
          </select>
        </label>
      </div>
      @if (editing()) {
        <div class="jmc" [id]="editorId" role="group" aria-label="Just my code patterns">
          <label class="col">Framework modules or functions (one per line, * wildcard, matched case-insensitively)
            <textarea rows="3" aria-label="Framework patterns" [value]="state.frameworkPatterns().join('\n')" #pat></textarea>
          </label>
          <label class="inline"><input type="checkbox" [checked]="state.useFrameFlags()" (change)="state.setUseFrameFlags($any($event.target).checked)" /> Also use the importer's framework flags</label>
          <div class="row-btns">
            <button class="btn" type="button" (click)="savePatterns(pat.value)">Apply patterns</button>
            <button class="btn" type="button" (click)="editing.set(false)">Close</button>
          </div>
        </div>
      }
    }
  `,
  styles: [
    PROFILING_STYLES,
    `
      :host { display: block; flex: none; }
      .jmc { display: flex; flex-direction: column; gap: 6px; padding: 6px 12px 8px; border-bottom: 1px solid var(--border); background: var(--bg-2); }
      .col { display: flex; flex-direction: column; gap: 4px; }
      textarea { font: inherit; font-family: var(--font-mono); font-size: 12px; color: var(--text); background: var(--bg-3); border: 1px solid var(--border-2); border-radius: 4px; padding: 4px 6px; max-width: 520px; }
      .row-btns { display: flex; gap: 6px; }
    `,
  ],
})
export class ProfileBar {
  readonly showMetric = input(true);
  protected readonly active = inject(ActiveProfile);
  protected readonly captures = inject(CapturesStore);
  protected readonly state = inject(ProfileViewState);
  protected readonly editing = signal(false);
  protected readonly editorId = nextTagId('jmc-editor');
  protected readonly recursion = RECURSION;
  protected readonly ns = formatNs;
  protected readonly measurement = computed(() => {
    const m = this.active.metric();
    return this.active.profile()?.metrics.find((x) => x.id === m)?.measurement ?? null;
  });

  protected pickProfile(e: Event): void {
    this.state.setProfile(Number((e.target as HTMLSelectElement).value));
  }

  protected pickMetric(e: Event): void {
    this.state.setMetric((e.target as HTMLSelectElement).value as MetricId);
  }

  protected savePatterns(text: string): void {
    this.state.setFrameworkPatterns(cleanPatterns(text.split('\n')));
    this.state.setJustMyCode(true);
  }
}

export function nextTagId(prefix: string): string {
  return `${prefix}-${++tagIds}`;
}
