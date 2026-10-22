import { ChangeDetectionStrategy, Component, computed, DestroyRef, inject, input, linkedSignal, OnInit, signal } from '@angular/core';
import { JobFollow } from '../../jobs/job-follow';
import { JobWait } from '../../jobs/job-wait';
import { DIALOG_REF } from '../../ui/dialog.service';
import { AiBackend, toAiError } from '../ai-backend';
import { ProposeKind } from '../ai-launcher';
import { AiStartRequest, Proposal, SampleSource } from '../ai.types';
import { AiProposalCard } from '../proposals/proposal-card';

const PROMPTS: Record<ProposeKind, { label: string; placeholder: string }> = {
  pipeline: {
    label: $localize`:@@ai.propose.pipeline.label:What should the pipeline do?`,
    placeholder: $localize`:@@ai.propose.pipeline.placeholder:e.g. drop health checks and mask customer e-mails`,
  },
  markers: {
    label: $localize`:@@ai.propose.markers.label:Which events should be marked?`,
    placeholder: $localize`:@@ai.propose.markers.placeholder:e.g. timeouts and payment failures`,
  },
  extractors: {
    label: $localize`:@@ai.propose.extractors.label:Which values should be extracted?`,
    placeholder: $localize`:@@ai.propose.extractors.placeholder:e.g. the order id and the duration in ms`,
  },
  parser: { label: $localize`:@@ai.propose.parser.label:Anything special about this file? (optional)`, placeholder: '' },
};

/**
 * "Propose pipeline/markers/extractors/parser": the model proposes, the engine validates it on a sample of
 * the file and previews the effect; Apply saves through the profile's normal (validated, undoable) save.
 */
@Component({
  selector: 'debrief-ai-propose-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiProposalCard, JobWait],
  template: `
    <p class="muted small">{{ sourceLine() }}</p>
    @if (chooseKind()) {
      <label for="ai-propose-kind" i18n="verb|@@ai.propose.kindLabel">Propose</label>
      <select id="ai-propose-kind" class="select" [value]="current()" (change)="pick(asValue($event))">
        <option value="pipeline" i18n="@@ai.propose.kind.pipeline">Pipeline steps</option>
        <option value="markers" i18n="@@ai.propose.kind.markers">Markers</option>
        <option value="extractors" i18n="@@ai.propose.kind.extractors">Extractors</option>
      </select>
    }
    @if (current() !== 'parser') {
      <label for="ai-propose-goal">{{ prompt().label }}</label>
      <textarea id="ai-propose-goal" class="input" rows="2" maxlength="2000" [attr.placeholder]="prompt().placeholder" [value]="goal()" (input)="goal.set(asValue($event))"></textarea>
    }
    <div class="row">
      <button type="button" class="btn primary" [disabled]="(current() !== 'parser' && !goal().trim()) || busy()" (click)="propose()">{{ proposeLabel() }}</button>
    </div>
    @if (jobId(); as id) {
      <debrief-job-wait [jobId]="id" [busyText]="busyText" />
    }
    @if (error(); as e) {
      <p class="err" role="alert">{{ e }}</p>
    }
    @if (proposal(); as p) {
      <debrief-ai-proposal-card [proposal]="p" [state]="state()" (applied)="state.set('applied')" (dismissed)="proposal.set(null)" />
    }
    <div class="foot"><button type="button" class="btn ghost" (click)="ref.close()" i18n="@@ai.propose.close">Close</button></div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 6px; }
    textarea { resize: vertical; font-family: inherit; }
    .row { display: flex; justify-content: flex-end; }
    .small { margin: 0; font-size: var(--fs-sm); }
    .err { margin: 0; color: var(--danger-text, var(--lvl-error)); }
    .foot { display: flex; justify-content: flex-end; }
  `,
})
export class AiProposeDialog implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(AiBackend);
  private readonly follow = inject(JobFollow);
  private readonly destroyRef = inject(DestroyRef);
  readonly kind = input<ProposeKind>('pipeline');
  readonly source = input<SampleSource>({});
  readonly initialGoal = input('');
  /** Let the user pick pipeline, markers or extractors (opened from the profile editor). */
  readonly chooseKind = input(false);
  protected readonly current = linkedSignal<ProposeKind>(() => this.kind());
  protected readonly goal = signal('');
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly proposal = signal<Proposal | null>(null);
  protected readonly state = signal<'open' | 'applied'>('open');
  /** The AI job of the running request (the dialog follows it; closing leaves it running). */
  protected readonly jobId = signal<number | null>(null);
  protected readonly busyText = $localize`:@@ai.propose.busy:The local model proposes; the engine checks the proposal on the sample…`;
  protected readonly prompt = computed(() => PROMPTS[this.current()]);
  protected readonly proposeLabel = computed(() =>
    this.busy() ? $localize`:@@ai.propose.proposing:Proposing…` : $localize`:verb, button that asks the AI for a proposal|@@ai.propose.button:Propose`,
  );
  protected readonly sourceLine = computed(() => {
    const s = this.source();
    if (s.sample?.trim()) {
      const n = s.sample.split(/\r?\n/).length;
      return $localize`:@@ai.propose.sample.given:Sample: ${n}:count: lines you provided.`;
    }
    if (s.sourceId != null && s.fileName) {
      return $localize`:@@ai.propose.sample.source:File: ${s.fileName}:name:. The engine checks each draft on it; the model sees only parts of it.`;
    }
    if (s.recordId != null) return $localize`:@@ai.propose.sample.event:Sample: the lines around event ${s.recordId}:id: in its file.`;
    return $localize`:@@ai.propose.sample.none:Select an event first: its file is the sample.`;
  });

  ngOnInit(): void {
    this.goal.set(this.initialGoal());
  }

  protected asValue(e: Event): string {
    return (e.target as HTMLTextAreaElement | HTMLSelectElement).value;
  }

  protected pick(kind: string): void {
    if (kind === 'pipeline' || kind === 'markers' || kind === 'extractors') {
      this.current.set(kind);
      this.proposal.set(null);
    }
  }

  private request(): AiStartRequest {
    const goal = this.goal().trim();
    const src = this.source();
    const recordId = src.recordId ?? null;
    const sample = src.sample ?? null;
    const op = this.current();
    return op === 'parser'
      ? { op, recordId, sample, fileName: src.fileName ?? null, ...(src.sourceId != null ? { sourceId: src.sourceId } : {}) }
      : { op, goal, recordId, sample };
  }

  /** Starts the proposal as an AI job and shows it here when it is done (if the dialog is still open). */
  async propose(): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    this.proposal.set(null);
    this.state.set('open');
    try {
      const result = await this.follow.run(() => this.backend.start(this.request()), this.destroyRef, (id) => this.jobId.set(id));
      if (result?.type === 'proposal') this.proposal.set(result.proposal as Proposal);
    } catch (e) {
      this.error.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
      this.jobId.set(null);
    }
  }
}
