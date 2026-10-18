import { ChangeDetectionStrategy, Component, computed, inject, input, OnInit, signal } from '@angular/core';
import { DIALOG_REF } from '../../ui/dialog.service';
import { AiBackend, toAiError } from '../ai-backend';
import { Narrative } from '../ai.types';
import { AiCitations } from '../chat/citation-actions';
import { isNotFound } from '../chat/markdown';
import { AiRichText } from '../chat/rich-text';

export type NarrativeKind = 'explain' | 'summarize' | 'rootCauses';

const WORKING: Record<NarrativeKind, string> = {
  explain: $localize`:@@ai.narrative.working.explain:Reading the event and the lines around it…`,
  summarize: $localize`:@@ai.narrative.working.summarize:Reading what happened in this range…`,
  rootCauses: $localize`:@@ai.narrative.working.rootCauses:Reading the health check and the exceptions…`,
};

/** "Explain this event", "Summarize this range" and "Root causes": a cited answer from the local model. */
@Component({
  selector: 'imt-ai-narrative-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiRichText],
  template: `
    @if (busy()) {
      <p class="muted" role="status" aria-live="polite">{{ working() }}</p>
      <div class="foot"><button type="button" class="btn" (click)="cancel()" i18n="@@ai.narrative.cancel">Cancel</button></div>
    } @else if (error(); as e) {
      <p class="err" role="alert">{{ e }}</p>
      <div class="foot">
        <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@ai.narrative.close">Close</button>
        <button type="button" class="btn primary" (click)="run()" i18n="@@ai.narrative.retry">Retry</button>
      </div>
    } @else if (result(); as r) {
      <div class="answer" [class.neutral]="notFound()" data-testid="ai-narrative">
        <imt-ai-rich-text [text]="r.text" [citations]="r.citations" (record)="jump($event)" (query)="applyQuery($event)" (link)="openWeb($event.url, $event.title)" />
      </div>
      @if (note(); as n) {
        <p class="note" role="status">{{ n }}</p>
      }
      <p class="muted small" i18n="@@ai.narrative.footnote">Written by the local model from the engine's results; links open the evidence.</p>
      <div class="foot"><button type="button" class="btn primary" (click)="ref.close()" i18n="@@ai.narrative.close">Close</button></div>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    .answer { max-height: 55vh; overflow-y: auto; }
    .answer.neutral { color: var(--muted); font-style: italic; }
    .err { margin: 0; color: var(--danger-text, var(--lvl-error)); }
    .note, .small { margin: 0; font-size: 12px; }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class AiNarrativeDialog implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(AiBackend);
  private readonly citations = inject(AiCitations);
  readonly kind = input<NarrativeKind>('explain');
  readonly recordId = input<number | null>(null);
  readonly from = input<number | null>(null);
  readonly to = input<number | null>(null);
  /** An answer already given (a finished AI job's result, opened from the Jobs pane or a notification). */
  readonly preset = input<Narrative | null>(null);

  protected readonly busy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly result = signal<Narrative | null>(null);
  protected readonly note = signal<string | null>(null);
  protected readonly working = computed(() => WORKING[this.kind()]);
  protected readonly notFound = computed(() => isNotFound(this.result()?.text ?? ''));

  ngOnInit(): void {
    const preset = this.preset();
    if (preset) this.result.set(preset);
    else void this.run();
  }

  private request(): Promise<Narrative> {
    switch (this.kind()) {
      case 'explain':
        return this.backend.explainEvent(this.recordId() ?? 0);
      case 'summarize':
        return this.backend.summarize(this.from() ?? 0, this.to() ?? 0);
      case 'rootCauses':
        return this.backend.rootCauses();
    }
  }

  async run(): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    try {
      this.result.set(await this.request());
    } catch (e) {
      const err = toAiError(e);
      this.error.set(err.code === 'cancelled' ? $localize`:@@ai.narrative.stopped:Stopped.` : err.message);
    } finally {
      this.busy.set(false);
    }
  }

  protected cancel(): void {
    void this.backend.cancel().catch(() => undefined);
  }

  protected async jump(id: number): Promise<void> {
    this.ref.close();
    await this.citations.record(id);
  }

  protected async applyQuery(ql: string): Promise<void> {
    const err = await this.citations.query(ql);
    if (err) this.note.set($localize`:@@ai.narrative.notApplied:Not applied: ${err}:error:`);
    else this.ref.close();
  }

  protected async openWeb(url: string, title: string): Promise<void> {
    const err = await this.citations.web(url, title);
    if (err) this.note.set(err);
  }
}
