import { ChangeDetectionStrategy, Component, inject, input, signal } from '@angular/core';
import { DIALOG_REF } from '../../ui/dialog.service';
import { Proposal } from '../ai.types';
import { AiProposalCard } from '../proposals/proposal-card';

/** A finished AI job's proposal (opened from the Jobs pane or a notification): review, then Apply or dismiss. */
@Component({
  selector: 'debrief-ai-proposal-result-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiProposalCard],
  template: `
    @if (!dismissed()) {
      <debrief-ai-proposal-card [proposal]="proposal()" [state]="state()" (applied)="state.set('applied')" (dismissed)="dismissed.set(true)" />
    } @else {
      <p class="muted" i18n="@@ai.proposalResult.dismissed">Dismissed.</p>
    }
    <div class="foot"><button type="button" class="btn ghost" (click)="ref.close()" i18n="@@ai.proposalResult.close">Close</button></div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    .foot { display: flex; justify-content: flex-end; }
  `,
})
export class AiProposalResultDialog {
  static readonly dialogId = 'ai-proposal-result';
  protected readonly ref = inject(DIALOG_REF);
  readonly proposal = input.required<Proposal>();
  protected readonly state = signal<'open' | 'applied' | 'dismissed'>('open');
  protected readonly dismissed = signal(false);
}
