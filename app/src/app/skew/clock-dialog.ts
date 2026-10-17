import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { DIALOG_FORM_STYLES } from '../context/dialog-styles';
import { formatFull } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { DIALOG_REF } from '../ui/dialog.service';
import { ClockSkewStore } from './clock-skew.store';
import { confidenceLabel, formatConfidence, formatOffset, parseOffset } from './skew-format';

/**
 * "Adjust clock…" for one source: a manual offset, alignment of the selected event on the clock
 * reference, or the engine's suggestion from shared correlation ids. The engine applies the offset.
 */
@Component({
  selector: 'imt-clock-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p>
      Shift the timestamps of <strong>{{ name() }}</strong> so its events line up with other machines.
      Current offset: <strong data-testid="current-offset">{{ fmt(current()) }}</strong>.
    </p>

    <section aria-labelledby="skew-manual">
      <h3 id="skew-manual">Manual offset</h3>
      <div class="row">
        <label class="lbl">
          Offset (seconds, or with units such as 2350ms or -1h 5s)
          <input
            type="text"
            data-testid="offset-input"
            [value]="text()"
            (input)="text.set($any($event.target).value)"
            (keydown.enter)="applyManual()"
            [attr.aria-invalid]="text() !== '' && parsed() === null"
            aria-describedby="skew-manual-hint"
          />
        </label>
        <button type="button" class="btn primary" [disabled]="parsed() === null || skew.busy()" (click)="applyManual()">Apply</button>
        @if (current() !== 0) {
          <button type="button" class="btn" [disabled]="skew.busy()" (click)="reset()">Reset to 0</button>
        }
      </div>
      <p id="skew-manual-hint" class="hint">
        @if (text() !== '' && parsed() === null) { <span class="err">Not an offset. Examples: 2.35, -1.5, +2350ms, -1h 5s.</span> }
        @else if (parsed() !== null) { Will shift by {{ fmt(parsed()!) }}. }
        @else { Positive values move the events later. }
      </p>
    </section>

    <section aria-labelledby="skew-align">
      <h3 id="skew-align">Align on an event</h3>
      @if (reference(); as ref) {
        <p class="small">Reference: <strong>{{ ref.sourceName }}</strong> · {{ time(ref.ts) }} · <span class="ellipsis">{{ ref.message }}</span></p>
        @if (target(); as t) {
          <p class="small">Same moment in this source: {{ time(t.ts) }} · <span class="ellipsis">{{ t.message }}</span></p>
          <button type="button" class="btn" data-testid="align" [disabled]="skew.busy()" (click)="align()">Align on these events</button>
        } @else {
          <p class="small faint">Select the matching event of {{ name() }} in the events grid, then open this dialog again.</p>
        }
      } @else {
        <p class="small faint">
          Select an event in another source and choose "Use as clock reference" in the inspector, then select the same real-world
          event in {{ name() }}.
        </p>
      }
    </section>

    <section aria-labelledby="skew-suggest">
      <h3 id="skew-suggest">Suggestion from correlation ids</h3>
      <div class="row">
        <button type="button" class="btn" data-testid="suggest" [disabled]="skew.suggesting()" (click)="suggest()">
          {{ skew.suggesting() ? 'Looking…' : 'Suggest' }}
        </button>
      </div>
      @if (skew.suggestion(); as result) {
        @if (mine(); as s) {
          <div class="suggestion" data-testid="suggestion">
            <p>
              Align with <strong>{{ sourceName(s.reference) }}</strong>: <strong>{{ fmt(s.suggestedMs) }}</strong>
              <span class="conf" [class]="'conf ' + label(s.confidence)">{{ label(s.confidence) }} confidence · {{ pct(s.confidence) }}</span>
            </p>
            <p class="small faint">{{ s.agreeing }} of {{ s.pairs }} events with a shared {{ result.fields.join(' / ') }} agree (spread {{ fmt(s.spreadMs).replace('+', '') }}).</p>
            <button type="button" class="btn primary" data-testid="apply-suggestion" [disabled]="skew.busy()" (click)="applySuggestion(s.suggestedMs)">Apply</button>
          </div>
        } @else if (result.reference === sourceId()) {
          <p class="small faint">This source is the reference others are aligned to.</p>
        } @else {
          <p class="small faint">Too few events share a {{ result.fields.join(' / ') }} with another source to suggest an offset.</p>
        }
      }
    </section>

    @if (skew.error(); as e) { <p class="err" role="alert">{{ e }}</p> }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="ref.close()">Close</button>
    </div>
  `,
  styles: [
    DIALOG_FORM_STYLES,
    `
      section { display: flex; flex-direction: column; gap: 6px; padding-top: 8px; border-top: 1px solid var(--border); }
      h3 { margin: 0; font-size: 12px; font-weight: 600; color: var(--text); }
      input { height: 26px; padding: 0 8px; border: 1px solid var(--border-2); border-radius: 5px; background: var(--bg); color: var(--text); font: inherit; }
      input[aria-invalid='true'] { border-color: var(--danger-text); }
      .hint, .small { font-size: 11.5px; }
      .ellipsis { display: inline-block; max-width: 320px; vertical-align: bottom; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .suggestion { display: flex; flex-direction: column; gap: 6px; align-items: flex-start; }
      .conf { margin-left: 6px; font-size: 11px; padding: 0 6px; border-radius: 8px; border: 1px solid var(--border-2); }
      .conf.high { border-color: var(--ok-text, var(--accent)); }
      .conf.low { border-color: var(--warn-text); color: var(--warn-text); }
    `,
  ],
})
export class ClockDialog {
  readonly sourceId = input.required<number>();

  protected readonly ref = inject(DIALOG_REF);
  protected readonly skew = inject(ClockSkewStore);
  private readonly store = inject(WorkspaceStore);

  protected readonly text = signal('');
  protected readonly parsed = computed(() => parseOffset(this.text()));
  private readonly source = computed(() => this.store.summary()?.sources.find((s) => s.id === this.sourceId()) ?? null);
  protected readonly name = computed(() => this.source()?.name ?? `source ${this.sourceId()}`);
  protected readonly current = computed(() => this.source()?.offsetMs ?? 0);
  /** The reference event, when it belongs to another source. */
  protected readonly reference = computed(() => {
    const r = this.skew.reference();
    return r && r.source !== this.sourceId() ? r : null;
  });
  /** The selected event, when it belongs to this source and has a time. */
  protected readonly target = computed(() => {
    const r = this.store.selected();
    return r && r.source === this.sourceId() && r.ts !== null ? { id: r.id, ts: r.ts, message: r.message } : null;
  });
  protected readonly mine = computed(() => this.skew.suggestion()?.suggestions.find((s) => s.source === this.sourceId()) ?? null);

  protected readonly fmt = formatOffset;
  protected readonly pct = formatConfidence;
  protected readonly label = confidenceLabel;

  constructor() {
    this.skew.clearError();
  }

  protected time(ts: number): string {
    return formatFull(ts, this.store.timezone());
  }

  protected sourceName(id: number): string {
    return this.store.summary()?.sources.find((s) => s.id === id)?.name ?? `source ${id}`;
  }

  protected applyManual(): Promise<void> {
    const ms = this.parsed();
    return ms === null ? Promise.resolve() : this.run({ kind: 'manual', source: this.sourceId(), offsetMs: ms });
  }

  protected reset(): Promise<void> {
    return this.run({ kind: 'manual', source: this.sourceId(), offsetMs: 0 });
  }

  protected align(): Promise<void> {
    const ref = this.reference();
    const target = this.target();
    return ref && target ? this.run({ kind: 'align', reference: ref.id, target: target.id }) : Promise.resolve();
  }

  protected applySuggestion(ms: number): Promise<void> {
    return this.run({ kind: 'manual', source: this.sourceId(), offsetMs: ms });
  }

  protected suggest(): Promise<void> {
    const ref = this.reference();
    return this.skew.suggest(ref ? ref.source : null);
  }

  private async run(spec: Parameters<InstanceType<typeof ClockSkewStore>['apply']>[0]): Promise<void> {
    if (await this.skew.apply(spec)) this.ref.close();
  }
}
