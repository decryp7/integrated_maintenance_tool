import { ChangeDetectionStrategy, Component, computed, inject, input, output, signal } from '@angular/core';
import { plural } from '../../i18n/locale';
import { toAiError } from '../ai-backend';
import { Proposal } from '../ai.types';
import { ProposalApply } from './proposal-apply';

const TITLES: Record<Proposal['kind'], string> = {
  regex: $localize`:@@ai.proposalCard.title.regex:Regular expression`,
  profile: $localize`:@@ai.proposalCard.title.profile:Parsing profile`,
  pipeline: $localize`:@@ai.proposalCard.title.pipeline:Pipeline steps`,
  markers: $localize`:@@ai.proposalCard.title.markers:Markers`,
  extractors: $localize`:@@ai.proposalCard.title.extractors:Extractors`,
  query: $localize`:@@ai.proposalCard.title.query:Query`,
};

const APPLY: Record<Proposal['kind'], string> = {
  regex: $localize`:@@ai.proposalCard.apply.regex:Add as highlight…`,
  profile: $localize`:@@ai.proposalCard.apply.profile:Save as profile`,
  pipeline: $localize`:@@ai.proposalCard.apply.addToProfile:Add to profile`,
  markers: $localize`:@@ai.proposalCard.apply.addToProfile:Add to profile`,
  extractors: $localize`:@@ai.proposalCard.apply.addToProfile:Add to profile`,
  query: $localize`:@@ai.proposalCard.apply.query:Apply query`,
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
    <section class="card" [attr.aria-label]="ariaLabel()" data-testid="ai-proposal">
      <header>
        <span class="kind">{{ title() }}</span>
        <span class="badge" i18n="@@ai.proposalCard.checked">checked by the engine</span>
        @if (state() !== 'open') {
          <span class="badge done">{{ stateLabel() }}</span>
        }
      </header>
      @switch (p().kind) {
        @case ('regex') {
          @if (regex(); as r) {
            <code class="mono block">{{ r.pattern }}</code>
            <p class="facts">{{ regexFacts(r) }}</p>
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
            <p class="facts" i18n="@@ai.proposalCard.profile.facts">{{ r.name }} · {{ r.importer ?? noImporter }} · parses {{ r.parsed }} entries, {{ r.withTimestamp }} with a timestamp</p>
            @if (r.preview.length) {
              <table class="preview">
                <thead><tr><th scope="col" i18n="@@ai.proposalCard.col.line">Line</th><th scope="col" i18n="log level|@@ai.proposalCard.col.level">Level</th><th scope="col" i18n="@@ai.proposalCard.col.message">Message</th></tr></thead>
                <tbody>@for (row of r.preview.slice(0, 5); track $index) {<tr><td>{{ row.line }}</td><td>{{ row.level }}</td><td class="msg">{{ row.message }}</td></tr>}</tbody>
              </table>
            }
            <details><summary i18n="@@ai.proposalCard.profileToml">Profile TOML</summary><pre class="mono">{{ r.toml }}</pre></details>
          }
        }
        @case ('pipeline') {
          @if (pipeline(); as r) {
            <p class="facts" i18n="@@ai.proposalCard.pipeline.facts">On {{ r.sampleTotal }} sample events: keeps {{ r.kept }}, drops {{ r.dropped }}, changes {{ r.changed }}</p>
            <ul class="steps">@for (s of r.steps; track s.index) {<li><span class="mono">{{ s.step }}</span> <span class="muted">{{ s.input }} → {{ s.output }}</span></li>}</ul>
            @for (ex of r.examples.slice(0, 3); track $index) {
              <p class="example mono"><span class="before">{{ ex.before }}</span> → <span>{{ ex.after }}</span></p>
            }
            <details><summary i18n="@@ai.proposalCard.stepsToml">Steps as TOML</summary><pre class="mono">{{ r.toml }}</pre></details>
          }
        }
        @case ('markers') {
          @if (markers(); as r) {
            <p class="facts" i18n="@@ai.proposalCard.markers.checked">Checked on {{ r.sampled }} sample events</p>
            <ul class="steps">@for (m of r.markers; track m.kind) {<li i18n="@@ai.proposalCard.markers.item">{{ m.label }} <span class="mono muted">({{ m.kind }})</span> · {m.matches, plural, =1 {{{ m.matches }} match} other {{{ m.matches }} matches}}</li>}</ul>
            <details><summary i18n="@@ai.proposalCard.markersToml">Markers as TOML</summary><pre class="mono">{{ r.toml }}</pre></details>
          }
        }
        @case ('extractors') {
          @if (extractors(); as r) {
            <p class="facts" i18n="@@ai.proposalCard.extractors.checked">Checked on {{ r.sampled }} sample events</p>
            <ul class="steps">@for (x of r.extractors; track x.name) {<li><ng-container i18n="@@ai.proposalCard.extractors.item"><span class="mono">{{ x.name }}</span>: {{ x.fields.join(', ') }} · {x.matches, plural, =1 {{{ x.matches }} match} other {{{ x.matches }} matches}}</ng-container>@if (x.examples[0]; as ex) {<span class="muted mono" i18n="@@ai.proposalCard.extractors.example"> e.g. {{ example(ex) }}</span>}</li>}</ul>
            <details><summary i18n="@@ai.proposalCard.extractorsToml">Extractors as TOML</summary><pre class="mono">{{ r.toml }}</pre></details>
          }
        }
        @case ('query') {
          @if (query(); as r) {
            <code class="mono block">{{ r.ql }}</code>
            <p class="facts">{{ r.explanation }}@if (r.estimatedMatches !== null) {<ng-container i18n="@@ai.proposalCard.query.results"> · {r.estimatedMatches, plural, =1 {{{ r.estimatedMatches }} result} other {{{ r.estimatedMatches }} results}} now</ng-container>}@if (r.repaired) {<ng-container i18n="@@ai.proposalCard.query.repaired"> · repaired after a syntax error</ng-container>}</p>
          }
        }
      }
      @if (rationale()) {
        <p class="why"><span class="muted" i18n="@@ai.proposalCard.why">Why:</span> {{ rationale() }}</p>
      }
      @if (error(); as e) {
        <p class="err" role="alert">{{ e }}</p>
      }
      @if (message(); as m) {
        <p class="ok" role="status">{{ m }}</p>
      }
      @if (state() === 'open') {
        <div class="actions">
          <button type="button" class="btn ghost" [disabled]="busy()" (click)="dismissed.emit()" i18n="@@ai.proposalCard.dismiss">Dismiss</button>
          <button type="button" class="btn primary" [disabled]="busy()" (click)="apply()">{{ applyText() }}</button>
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
  protected readonly ariaLabel = computed(() => $localize`:@@ai.proposalCard.ariaLabel:${this.title()}:title: proposal`);
  protected readonly stateLabel = computed(() =>
    this.state() === 'applied' ? $localize`:@@ai.proposalCard.state.applied:applied` : $localize`:@@ai.proposalCard.state.dismissed:dismissed`,
  );
  protected readonly applyText = computed(() => (this.busy() ? $localize`:@@ai.proposalCard.applying:Applying…` : this.applyLabel() || this.defaultApply()));
  protected readonly noImporter = $localize`:@@ai.proposalCard.noImporter:no importer`;
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

  protected regexFacts(r: { matchCount: number; total: number; source: string }): string {
    const m = r.matchCount;
    const t = r.total;
    const inferred = r.source === 'inferred';
    return plural(t, {
      one: inferred
        ? $localize`:@@ai.proposalCard.regex.facts.oneInferred:Matches ${m}:matches: of ${t}:total: example · the engine's own inference`
        : $localize`:@@ai.proposalCard.regex.facts.one:Matches ${m}:matches: of ${t}:total: example`,
      other: inferred
        ? $localize`:@@ai.proposalCard.regex.facts.otherInferred:Matches ${m}:matches: of ${t}:total: examples · the engine's own inference`
        : $localize`:@@ai.proposalCard.regex.facts.other:Matches ${m}:matches: of ${t}:total: examples`,
    });
  }

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
      this.error.set($localize`:@@ai.proposalCard.notApplied:Not applied: ${toAiError(e).message}:error:`);
    } finally {
      this.busy.set(false);
    }
  }
}

function narrow<K extends Proposal['kind']>(p: Proposal, kind: K): Extract<Proposal, { kind: K }> | null {
  return p.kind === kind ? (p as Extract<Proposal, { kind: K }>) : null;
}
