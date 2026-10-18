import { ChangeDetectionStrategy, Component, computed, input, output, signal } from '@angular/core';
import { ApprovalDecision } from '../ai.types';
import { ApprovalView } from './chat-model';

const DONE: Record<string, string> = {
  allowed: $localize`:@@ai.approval.done.allowed:Sent`,
  edited: $localize`:@@ai.approval.done.edited:Sent (edited)`,
  allowedForChat: $localize`:@@ai.approval.done.allowedForChat:Sent; further searches in this chat are allowed`,
  denied: $localize`:@@ai.approval.done.denied:Not sent`,
  sending: $localize`:@@ai.approval.done.sending:Sending your answer…`,
};

/**
 * Asks before anything leaves the computer: the exact search text (or the page's address and site), with
 * Send / Edit / Deny and "allow searches for this chat".
 */
@Component({
  selector: 'imt-ai-approval',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <section class="ask" role="group" [attr.aria-label]="heading()" data-testid="ai-approval">
      <p class="q"><strong>{{ heading() }}</strong> <span class="muted" i18n="@@ai.approval.via">via {{ a().provider }}</span></p>
      @if (editing()) {
        <label class="sr-only" [attr.for]="inputId()" i18n="@@ai.approval.searchTextLabel">Search text to send</label>
        <input class="input mono" [id]="inputId()" [value]="draft()" (input)="draft.set(asValue($event))" (keydown.enter)="sendEdited()" maxlength="500" />
      } @else {
        <code class="mono text">{{ a().text }}</code>
      }
      @if (a().domain) {
        <p class="muted small" i18n="@@ai.approval.site">Site: {{ a().domain }}</p>
      }
      @if (a().state === 'pending') {
        <p class="muted small" i18n="@@ai.approval.onlyThisSent">Only this text is sent. Nothing else from your logs leaves this computer.</p>
        <div class="actions">
          @if (editing()) {
            <button type="button" class="btn ghost" (click)="editing.set(false)" i18n="@@ai.approval.back">Back</button>
            <button type="button" class="btn primary" [disabled]="!draft().trim()" (click)="sendEdited()" i18n="@@ai.approval.sendEdited">Send edited</button>
          } @else {
            <button type="button" class="btn ghost" (click)="decide.emit({ kind: 'deny' })" i18n="@@ai.approval.deny">Deny</button>
            @if (a().editable) {
              <button type="button" class="btn ghost" (click)="startEdit()" i18n="@@ai.approval.edit">Edit</button>
            }
            @if (a().kind === 'search') {
              <button type="button" class="btn ghost" (click)="decide.emit({ kind: 'allowSearchesInChat' })" i18n="@@ai.approval.allowForChat">Allow for this chat</button>
            }
            <button type="button" class="btn primary" (click)="decide.emit({ kind: 'allow' })" i18n="@@ai.approval.send">Send</button>
          }
        </div>
      } @else {
        <p class="result small" role="status">{{ result() }}</p>
      }
    </section>
  `,
  styles: `
    :host { display: block; }
    .ask { border: 1px solid var(--ann-warning-border, var(--border)); background: var(--ann-warning, var(--bg-1)); border-radius: 8px; padding: 8px 10px; display: flex; flex-direction: column; gap: 6px; }
    .q { margin: 0; }
    .text { display: block; background: var(--code-bg); padding: 4px 6px; border-radius: 4px; overflow-wrap: anywhere; }
    .small { margin: 0; font-size: 12px; }
    .actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 6px; }
    .result { color: var(--text-2); }
  `,
})
export class AiApprovalPrompt {
  readonly approval = input.required<ApprovalView>();
  readonly decide = output<ApprovalDecision>();
  protected readonly editing = signal(false);
  protected readonly draft = signal('');
  protected readonly a = computed(() => this.approval());
  protected readonly heading = computed(() => (this.a().kind === 'search' ? $localize`:@@ai.approval.heading.search:Search the web for:` : $localize`:@@ai.approval.heading.page:Read this web page:`),
  );
  protected readonly inputId = computed(() => `ai-approval-edit-${this.a().requestId}`);
  protected readonly result = computed(() => DONE[this.a().state] ?? '');

  protected asValue(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }

  protected startEdit(): void {
    this.draft.set(this.a().text);
    this.editing.set(true);
  }

  protected sendEdited(): void {
    const text = this.draft().trim();
    if (!text) return;
    this.editing.set(false);
    this.decide.emit({ kind: 'allowEdited', text });
  }
}
