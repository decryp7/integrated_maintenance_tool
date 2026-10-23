import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { DIALOG_REF } from '../../ui/dialog.service';
import { Narrative } from '../ai.types';
import { AiCitations } from '../chat/citation-actions';
import { isNotFound } from '../chat/markdown';
import { AiRichText } from '../chat/rich-text';

/**
 * The answer of "Explain this event", "Summarize this range" or "Root causes", with its citations. The request runs
 * in the background as an AI job; this opens from its notification or the Jobs pane (View) once the answer is ready.
 */
@Component({
  selector: 'debrief-ai-narrative-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiRichText],
  template: `
    <div class="answer" [class.neutral]="notFound()" data-testid="ai-narrative">
      <debrief-ai-rich-text [text]="preset().text" [citations]="preset().citations" (record)="jump($event)" (query)="applyQuery($event)" (link)="openWeb($event.url, $event.title)" />
    </div>
    @if (note(); as n) {
      <p class="note" role="status">{{ n }}</p>
    }
    <p class="muted small" i18n="@@ai.narrative.footnote">Written by the local model from the engine's results; links open the evidence.</p>
    <div class="foot"><button type="button" class="btn primary" (click)="ref.close()" i18n="@@ai.narrative.close">Close</button></div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    .answer { max-height: 55vh; overflow-y: auto; }
    .answer.neutral { color: var(--muted); font-style: italic; }
    .note, .small { margin: 0; font-size: var(--fs-sm); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class AiNarrativeDialog {
  static readonly dialogId = 'ai-narrative';
  protected readonly ref = inject(DIALOG_REF);
  private readonly citations = inject(AiCitations);
  /** The finished request's answer (its job's result). */
  readonly preset = input.required<Narrative>();

  protected readonly note = signal<string | null>(null);
  protected readonly notFound = computed(() => isNotFound(this.preset().text));

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
