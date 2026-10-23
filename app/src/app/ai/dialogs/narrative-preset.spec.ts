import { Type } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { MockAiBackend } from '../../core/mock/mock-ai-backend';
import { DIALOG_REF } from '../../ui/dialog.service';
import { buttonByText, el, settle } from '../../../testing/helpers';
import { AiBackend } from '../ai-backend';
import { Narrative, Proposal } from '../ai.types';
import { AiNarrativeDialog } from './narrative-dialog';
import { AiProposalResultDialog } from './proposal-result-dialog';

function open<T>(component: Type<T>, inputs: Record<string, unknown>) {
  const backend = new MockAiBackend({ stepMs: 0 });
  const ref = { close: vi.fn(), closed: new Promise(() => undefined) };
  TestBed.configureTestingModule({
    providers: [...provideAnalysisTesting({ context: new FakeContext() }), { provide: AiBackend, useValue: backend }, { provide: DIALOG_REF, useValue: ref }],
  });
  const f = TestBed.createComponent(component);
  for (const [k, v] of Object.entries(inputs)) f.componentRef.setInput(k, v);
  return { f, backend, ref };
}

const preset: Narrative = {
  text: 'The gateway timed out after 30 s.',
  citations: [],
  evidence: { recordIds: [], queries: [] },
  usage: { inputTokens: 1, outputTokens: 1 },
};

describe('AiNarrativeDialog preset', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('shows the given answer at once and closes', async () => {
    const { f, ref } = open(AiNarrativeDialog, { preset });
    f.detectChanges();
    expect(el(f, '[data-testid=ai-narrative]').textContent).toContain('The gateway timed out after 30 s.');
    await settle(f);
    buttonByText(f, 'Close').click();
    expect(ref.close).toHaveBeenCalled();
  });
});

describe('AiProposalResultDialog', () => {
  afterEach(() => TestBed.resetTestingModule());

  const proposal = { kind: 'query', ql: 'level >= error', explanation: 'errors', estimatedMatches: 3, repaired: false } as unknown as Proposal;

  it('shows the proposal card and closes', async () => {
    const { f, ref } = open(AiProposalResultDialog, { proposal });
    f.detectChanges();
    expect(f.nativeElement.querySelector('debrief-ai-proposal-card')).toBeTruthy();
    buttonByText(f, 'Close').click();
    expect(ref.close).toHaveBeenCalled();
  });

  it('shows Dismissed after the card is dismissed', async () => {
    const { f } = open(AiProposalResultDialog, { proposal });
    f.detectChanges();
    buttonByText(f, 'Dismiss').click();
    f.detectChanges();
    expect(f.nativeElement.querySelector('debrief-ai-proposal-card')).toBeNull();
    expect(f.nativeElement.textContent).toContain('Dismissed.');
  });
});
