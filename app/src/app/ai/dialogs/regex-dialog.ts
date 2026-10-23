import { ChangeDetectionStrategy, Component, computed, DestroyRef, inject, input, OnInit, signal } from '@angular/core';
import { JobFollow } from '../../jobs/job-follow';
import { JobWait } from '../../jobs/job-wait';
import { DIALOG_REF } from '../../ui/dialog.service';
import { AiBackend, toAiError } from '../ai-backend';
import { Proposal, RegexProposal } from '../ai.types';
import { AiProposalCard } from '../proposals/proposal-card';

const MAX_LINES = 50;

/**
 * "Suggest regex": the model proposes a pattern for example lines, the engine checks it against them.
 * In a host's regex field (infer dialog, format wizard) the dialog closes with the pattern; from the
 * record menu (`highlight`) it is offered as a highlight rule.
 */
@Component({
  selector: 'debrief-ai-regex-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiProposalCard, JobWait],
  template: `
    <label for="ai-regex-lines" i18n="@@ai.regex.linesLabel">Example lines (one per line)</label>
    <textarea id="ai-regex-lines" class="input mono" rows="5" [value]="text()" (input)="text.set(asValue($event))"></textarea>
    <label for="ai-regex-intent" i18n="@@ai.regex.intentLabel">What should it capture? (optional)</label>
    <input id="ai-regex-intent" class="input" maxlength="500" placeholder="e.g. the order id and the duration" i18n-placeholder="@@ai.regex.intentPlaceholder" [value]="intentText()" (input)="intentText.set(asValue($event))" />
    <div class="row">
      <span class="muted small" i18n="@@ai.regex.lineCount">{lineCount(), plural, =1 {{{ lineCount() }} line} other {{{ lineCount() }} lines}}</span>
      <button type="button" class="btn primary" [disabled]="!lineCount() || busy()" (click)="suggest()">{{ suggestLabel() }}</button>
    </div>
    @if (jobId(); as id) {
      <debrief-job-wait [jobId]="id" [busyText]="busyText" />
    }
    @if (error(); as e) {
      <p class="err" role="alert">{{ e }}</p>
    }
    @if (proposal(); as p) {
      <debrief-ai-proposal-card
        [proposal]="p"
        [customApply]="!highlight()"
        [applyLabel]="highlight() ? '' : usePatternLabel"
        (applyRequested)="use($event)"
        (applied)="ref.close()"
        (dismissed)="proposal.set(null)"
      />
    }
    <div class="foot"><button type="button" class="btn ghost" (click)="ref.close()" i18n="@@ai.regex.close">Close</button></div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 6px; }
    textarea { resize: vertical; font-size: var(--fs-sm); }
    .row { display: flex; justify-content: space-between; align-items: center; }
    .small { font-size: var(--fs-sm); }
    .err { margin: 0; color: var(--danger-text, var(--lvl-error)); }
    .foot { display: flex; justify-content: flex-end; }
  `,
})
export class AiRegexDialog implements OnInit {
  static readonly dialogId = 'ai-regex';
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(AiBackend);
  private readonly follow = inject(JobFollow);
  private readonly destroyRef = inject(DestroyRef);
  readonly lines = input<string[]>([]);
  /** The AI job of the running request (the dialog follows it; closing leaves it running). */
  protected readonly jobId = signal<number | null>(null);
  protected readonly busyText = $localize`:@@ai.regex.busy:The local model suggests a pattern; the engine checks it on the lines…`;
  readonly intent = input('');
  readonly highlight = input(false);
  protected readonly text = signal('');
  protected readonly intentText = signal('');
  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly proposal = signal<RegexProposal | null>(null);
  protected readonly lineList = computed(() =>
    this.text()
      .split(/\r?\n/)
      .filter((l) => l.trim())
      .slice(0, MAX_LINES),
  );
  protected readonly lineCount = computed(() => this.lineList().length);
  protected readonly usePatternLabel = $localize`:@@ai.regex.usePattern:Use this pattern`;
  protected readonly suggestLabel = computed(() => (this.busy() ? $localize`:@@ai.regex.suggesting:Suggesting…` : $localize`:@@ai.regex.suggest:Suggest regex`));


  ngOnInit(): void {
    this.text.set(this.lines().slice(0, MAX_LINES).join('\n'));
    this.intentText.set(this.intent());
  }

  protected asValue(e: Event): string {
    return (e.target as HTMLInputElement | HTMLTextAreaElement).value;
  }

  async suggest(): Promise<void> {
    if (!this.lineCount()) return;
    this.busy.set(true);
    this.error.set(null);
    this.proposal.set(null);
    const request = { op: 'regex' as const, lines: this.lineList(), intent: this.intentText().trim() || null };
    try {
      const result = await this.follow.run(() => this.backend.start(request), this.destroyRef, (id) => this.jobId.set(id));
      if (result?.type === 'proposal') this.proposal.set(result.proposal as RegexProposal);
    } catch (e) {
      this.error.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
      this.jobId.set(null);
    }
  }

  protected use(p: Proposal): void {
    if (p.kind === 'regex') this.ref.close(p.pattern);
  }
}
