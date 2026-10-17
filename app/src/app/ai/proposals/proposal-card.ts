import { ChangeDetectionStrategy, Component, computed, inject, input, output, signal } from '@angular/core';
import { toAiError } from '../ai-backend';
import { Proposal } from '../ai.types';
import { ProposalApply } from './proposal-apply';

const TITLES: Record<Proposal['kind'], string> = {
  regex: 'Regular expression',
  profile: 'Parsing profile',
  pipeline: 'Pipeline steps',
  markers: 'Markers',
  extractors: 'Extractors',
  query: 'Query',
};

const APPLY: Record<Proposal['kind'], string> = {
  regex: 'Add as highlight…',
  profile: 'Save as profile',
  pipeline: 'Add to profile',
  markers: 'Add to profile',
  extractors: 'Add to profile',
  query: 'Apply query',
};

/**
 * One engine-validated proposal with the engine's own evidence (matches, preview rows, kept/dropped/changed,
 * explanation). Apply goes through the normal validated save paths (`ProposalApply`); nothing is applied
 * without the click. With `customApply` the host handles Apply itself (e.g. "Use this pattern").
 */
@Component({
  selector: 'imt-ai-proposal-card',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <section class="card" [attr.aria-label]="title() + ' proposal'" data-testid="ai-proposal">
      <header>
        <span class="kind">{{ title() }}</span>
        <span class="badge">checked by the engine</span>
        @if (state() !== 'open') {
          <span class="badge done">{{ state() === 'applied' ? 'applied' : 'dismissed' }}</span>
        }
      </header>
      @switch (p().kind) {
        @case ('regex') {
          @if (regex(); as r) {
            <code class="mono block">{{ r.pattern }}</code>
            <p class="facts">Matches {{ r.matchCount }} of {{ r.total }} example{{ r.total === 1 ? '' : 's' }}{{ r.source === 'inferred' ? ' · the engine\\'s own inference' : '' }}</p>
            @if (r.groups.length) {
              <p class="chips">@for (g of r.groups; track g.name) {<span class="chip mono">{{ g.name }}: {{ g.kind }}</span>}</p>
            }
            @if (r.explanation.length) {
              <ul class="explain">@for (n of r.explanation; track $index) {<li [style.padding-left.px]="n.depth * 14">{{ n.text }}</li>}</ul>
            }
          }
        }
        @case ('profile') {
          @if (profile(); as r) {
            <p class="facts">{{ r.name }} · {{ r.importer ?? 'no importer' }} · parses {{ r.parsed }} entries, {{ r.withTimestamp }} with a timestamp</p>
            @if (r.preview.length) {
              <table class="preview">
                <thead><tr><th scope="col">Line</th><th scope="col">Level</th><th scope="col">Message</th></tr></thead>
                <tbody>@for (row of r.preview.slice(0, 5); track $index) {<tr><td>{{ row.line }}</td><td>{{ row.level }}</td><td class="msg">{{ row.message }}</td></tr>}</tbody>
              </table>
            }
            <details><summary>Profile TOML</summary><pre class="mono">{{ r.toml }}</pre></details>
          }
        }
        @case ('pipeline') {
          @if (pipeline(); as r) {
            <p class="facts">On {{ r.sampleTotal }} sample events: keeps {{ r.kept }}, drops {{ r.dropped }}, changes {{ r.changed }}</p>
            <ul class="steps">@for (s of r.steps; track s.index) {<li><span class="mono">{{ s.step }}</span> <span class="muted">{{ s.input }} → {{ s.output }}</span></li>}</ul>
            @for (ex of r.examples.slice(0, 3); track $index) {
              <p class="example mono"><span class="before">{{ ex.before }}</span> → <span>{{ ex.after }}</span></p>
            }
            <details><summary>Steps as TOML</summary><pre class="mono">{{ r.toml }}</pre></details>
          }
        }
        @case ('markers') {
          @if (markers(); as r) {
            <p class="facts">Checked on {{ r.sampled }} sample events</p>
            <ul class="steps">@for (m of r.markers; track m.kind) {<li>{{ m.label }} <span class="mono muted">({{ m.kind }})</span> · {{ m.matches }} match{{ m.matches === 1 ? '' : 'es' }}</li>}</ul>
            <details><summary>Markers as TOML</summary><pre class="mono">{{ r.toml }}</pre></details>
          }
        }
        @case ('extractors') {
          @if (extractors(); as r) {
            <p class="facts">Checked on {{ r.sampled }} sample events</p>
            <ul class="steps">@for (x of r.extractors; track x.name) {<li><span class="mono">{{ x.name }}</span>: {{ x.fields.join(', ') }} · {{ x.matches }} match{{ x.matches === 1 ? '' : 'es' }}@if (x.examples.length) {<span class="muted mono"> e.g. {{ example(x.examples[0]) }}</span>}</li>}</ul>
            <details><summary>Extractors as TOML</summary><pre class="mono">{{ r.toml }}</pre></details>
          }
        }
        @case ('query') {
          @if (query(); as r) {
            <code class="mono block">{{ r.ql }}</code>
            <p class="facts">{{ r.explanation }}@if (r.estimatedMatches !== null) { · {{ r.estimatedMatches }} result{{ r.estimatedMatches === 1 ? '' : 's' }} now}@if (r.repaired) { · repaired after a syntax error}</p>
          }
        }
      }
      @if (rationale()) {
        <p class="why"><span class="muted">Why:</span> {{ rationale() }}</p>
      }
      @if (error(); as e) {
        <p class="err" role="alert">{{ e }}</p>
      }
      @if (message(); as m) {
        <p class="ok" role="status">{{ m }}</p>
      }
      @if (state() === 'open') {
        <div class="actions">
          <button type="button" class="btn ghost" [disabled]="busy()" (click)="dismissed.emit()">Dismiss</button>
          <button type="button" class="btn primary" [disabled]="busy()" (click)="apply()">{{ busy() ? 'Applying…' : applyLabel() || defaultApply() }}</button>
        </div>
      }
    </section>
  `,
  styles: `
    :host { display: block; }
    .card { border: 1px solid var(--border); border-radius: 8px; padding: 10px 12px; background: var(--bg-1); display: flex; flex-direction: column; gap: 6px; }
    header { display: flex; align-items: center; gap: 8px; }
    .kind { font-weight: 600; }
    .badge.done { color: var(--ok); }
    .block { display: block; background: var(--code-bg); padding: 6px 8px; border-radius: 4px; overflow-wrap: anywhere; font-size: 12px; }
    .facts, .why, .example { margin: 0; font-size: 12px; color: var(--text-2); }
    .chips { margin: 0; display: flex; flex-wrap: wrap; gap: 4px; }
    .explain, .steps { margin: 0; padding-left: 16px; font-size: 12px; color: var(--text-2); }
    .preview { border-collapse: collapse; font-size: 12px; width: 100%; }
    .preview th, .preview td { text-align: left; padding: 2px 6px; border-bottom: 1px solid var(--border); vertical-align: top; }
    .preview .msg { overflow-wrap: anywhere; }
    .before { text-decoration: line-through; color: var(--muted); }
    details summary { cursor: pointer; font-size: 12px; color: var(--muted); }
    pre { max-height: 200px; overflow: auto; background: var(--code-bg); padding: 6px; border-radius: 4px; font-size: 11px; margin: 4px 0 0; }
    .err { margin: 0; color: var(--danger-text, var(--lvl-error)); font-size: 12px; }
    .ok { margin: 0; color: var(--ok); font-size: 12px; }
    .actions { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class AiProposalCard {
  private readonly applier = inject(ProposalApply);
  readonly proposal = input.required<Proposal>();
  readonly state = input<'open' | 'applied' | 'dismissed'>('open');
  /** Overrides the Apply label (e.g. "Use this pattern"). */
  readonly applyLabel = input('');
  /** The host applies (it gets `applyRequested`); otherwise the card applies through `ProposalApply`. */
  readonly customApply = input(false);
  readonly applied = output<string>();
  readonly applyRequested = output<Proposal>();
  readonly dismissed = output<void>();

  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly message = signal<string | null>(null);

  protected readonly p = computed(() => this.proposal());
  protected readonly title = computed(() => TITLES[this.p().kind]);
  protected readonly defaultApply = computed(() => APPLY[this.p().kind]);
  protected readonly rationale = computed(() => {
    const p = this.p();
    return 'rationale' in p ? p.rationale : '';
  });
  protected readonly regex = computed(() => narrow(this.p(), 'regex'));
  protected readonly profile = computed(() => narrow(this.p(), 'profile'));
  protected readonly pipeline = computed(() => narrow(this.p(), 'pipeline'));
  protected readonly markers = computed(() => narrow(this.p(), 'markers'));
  protected readonly extractors = computed(() => narrow(this.p(), 'extractors'));
  protected readonly query = computed(() => narrow(this.p(), 'query'));

  protected example(e: Record<string, unknown>): string {
    return Object.entries(e)
      .map(([k, v]) => `${k}=${typeof v === 'string' ? v : JSON.stringify(v)}`)
      .join(', ');
  }

  async apply(): Promise<void> {
    if (this.customApply()) {
      this.applyRequested.emit(this.p());
      return;
    }
    this.busy.set(true);
    this.error.set(null);
    try {
      const r = await this.applier.apply(this.p());
      if (r.kind !== 'cancelled') {
        this.message.set(r.message);
        this.applied.emit(r.message);
      }
    } catch (e) {
      this.error.set(`Not applied: ${toAiError(e).message}`);
    } finally {
      this.busy.set(false);
    }
  }
}

function narrow<K extends Proposal['kind']>(p: Proposal, kind: K): Extract<Proposal, { kind: K }> | null {
  return p.kind === kind ? (p as Extract<Proposal, { kind: K }>) : null;
}
