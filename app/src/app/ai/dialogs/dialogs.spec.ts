import { Type } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { MockAiBackend } from '../../core/mock/mock-ai-backend';
import { DIALOG_REF } from '../../ui/dialog.service';
import { all, buttonByText, el, settle, typeInto } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
import { AiCitations } from '../chat/citation-actions';
import { ProposalApply } from '../proposals/proposal-apply';
import { AiNarrativeDialog } from './narrative-dialog';
import { AiProposeDialog } from './propose-dialog';
import { AiQueryDialog } from './query-dialog';
import { AiRegexDialog } from './regex-dialog';

function open<T>(component: Type<T>, inputs: Record<string, unknown> = {}) {
  const backend = new MockAiBackend({ stepMs: 0 });
  const ref = { close: vi.fn(), closed: new Promise(() => undefined) };
  TestBed.configureTestingModule({
    providers: [...provideAnalysisTesting({ context: new FakeContext() }), { provide: AiBackend, useValue: backend }, { provide: DIALOG_REF, useValue: ref }],
  });
  const f = TestBed.createComponent(component);
  for (const [k, v] of Object.entries(inputs)) f.componentRef.setInput(k, v);
  return { f, backend, ref, root: f.nativeElement as HTMLElement };
}

async function until(f: ComponentFixture<unknown>, cond: () => boolean) {
  for (let i = 0; i < 200 && !cond(); i++) {
    await settle();
    f.detectChanges();
  }
  expect(cond()).toBe(true);
}

describe('AiNarrativeDialog', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('explains an event and follows its evidence', async () => {
    const { f, backend, ref, root } = open(AiNarrativeDialog, { kind: 'explain', recordId: 42 });
    const explain = vi.spyOn(backend, 'explainEvent');
    f.detectChanges();
    expect(root.textContent).toContain('Reading the event');
    await until(f, () => !!root.querySelector('[data-testid=ai-narrative]'));
    expect(explain).toHaveBeenCalledWith(42);
    const cites = TestBed.inject(AiCitations);
    const record = vi.spyOn(cites, 'record').mockResolvedValue();
    const query = vi.spyOn(cites, 'query').mockResolvedValueOnce('bad').mockResolvedValueOnce(null);
    all<HTMLButtonElement>(f, 'button.cite.q')[0].click();
    await until(f, () => !!root.querySelector('.note'));
    expect(el(f, '.note').textContent).toContain('Not applied: bad');
    all<HTMLButtonElement>(f, 'button.cite.q')[0].click();
    await settle(f);
    expect(ref.close).toHaveBeenCalledTimes(1);
    all<HTMLButtonElement>(f, 'button.cite.rec')[0].click();
    await settle(f);
    expect(record).toHaveBeenCalledWith(42);
    expect(query).toHaveBeenCalledTimes(2);
    buttonByText(f, 'Close').click();
  });

  it('summarizes a range and finds root causes', async () => {
    const s = open(AiNarrativeDialog, { kind: 'summarize', from: Date.UTC(2026, 9, 14, 10), to: Date.UTC(2026, 9, 14, 11) });
    s.f.detectChanges();
    await until(s.f, () => !!s.root.querySelector('[data-testid=ai-narrative]'));
    expect(s.root.textContent).toContain('Between 10:00 and 11:00 UTC');
    TestBed.resetTestingModule();
    const r = open(AiNarrativeDialog, { kind: 'rootCauses' });
    r.f.detectChanges();
    await until(r.f, () => !!r.root.querySelector('[data-testid=ai-narrative]'));
    expect(r.root.textContent).toContain('Memory leak on WEB02');
  });

  it('defaults missing inputs, shows errors with Retry, cancels, and opens web sources', async () => {
    const { f, backend, root } = open(AiNarrativeDialog, {});
    const explain = vi.spyOn(backend, 'explainEvent').mockRejectedValueOnce(new AiError('engine', 'engine down'));
    const cancel = vi.spyOn(backend, 'cancel');
    f.detectChanges();
    buttonByText(f, 'Cancel').click();
    expect(cancel).toHaveBeenCalled();
    await until(f, () => !!root.querySelector('.err'));
    expect(explain).toHaveBeenCalledWith(0);
    expect(el(f, '.err').textContent).toBe('engine down');
    explain.mockRejectedValueOnce(new AiError('cancelled', 'cancelled'));
    buttonByText(f, 'Retry').click();
    await until(f, () => el(f, '.err').textContent === 'Stopped.');
    explain.mockResolvedValueOnce({ text: 'Not found in the logs. See https://w.example/a', citations: [{ kind: 'web', title: 'W', url: 'https://w.example/a' }], evidence: { recordIds: [], queries: [] }, usage: { inputTokens: 0, outputTokens: 0 } });
    buttonByText(f, 'Retry').click();
    await until(f, () => !!root.querySelector('.answer.neutral'));
    vi.spyOn(TestBed.inject(AiCitations), 'web').mockResolvedValueOnce('refused').mockResolvedValueOnce(null);
    all<HTMLButtonElement>(f, 'button.cite.web')[0].click();
    await until(f, () => !!root.querySelector('.note'));
    expect(el(f, '.note').textContent).toBe('refused');
    all<HTMLButtonElement>(f, 'button.cite.web')[0].click();
    await settle(f);
  });

  it('summarizes with defaults when the range is missing', async () => {
    const { f, backend } = open(AiNarrativeDialog, { kind: 'summarize' });
    const summarize = vi.spyOn(backend, 'summarize');
    f.detectChanges();
    await settle(f);
    expect(summarize).toHaveBeenCalledWith(0, 0);
  });
});

describe('AiQueryDialog', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('writes a query for a question and closes once applied', async () => {
    const { f, root, ref, backend } = open(AiQueryDialog, { initial: 'warnings please' });
    f.detectChanges();
    await until(f, () => !!root.querySelector('[data-testid=ai-proposal]'));
    expect(root.textContent).toContain('level = warning');
    vi.spyOn(TestBed.inject(ProposalApply), 'apply').mockResolvedValue({ kind: 'applied', message: 'ok' });
    buttonByText(f, 'Apply query').click();
    await settle(f);
    expect(ref.close).toHaveBeenCalledWith(true);
    buttonByText(f, 'Dismiss').click();
    f.detectChanges();
    expect(root.querySelector('[data-testid=ai-proposal]')).toBeNull();
    vi.spyOn(backend, 'nlToQuery').mockRejectedValueOnce(new AiError('noModel', 'set up first'));
    typeInto(el<HTMLInputElement>(f, 'input'), 'errors');
    f.detectChanges();
    el<HTMLFormElement>(f, 'form').dispatchEvent(new Event('submit'));
    await until(f, () => !!root.querySelector('.err'));
    expect(el(f, '.err').textContent).toBe('set up first');
  });

  it('waits for a question', async () => {
    const { f, backend } = open(AiQueryDialog);
    const ask = vi.spyOn(backend, 'nlToQuery');
    f.detectChanges();
    await f.componentInstance.ask();
    expect(ask).not.toHaveBeenCalled();
    expect(buttonByText(f, 'Write query').disabled).toBe(true);
  });
});

describe('AiRegexDialog', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('closes with the pattern for the host field', async () => {
    const lines = Array.from({ length: 60 }, (_, i) => `2026-10-14 09:00:0${i % 10} [INF] line ${i}`);
    const { f, root, ref, backend } = open(AiRegexDialog, { lines, intent: 'the level' });
    const propose = vi.spyOn(backend, 'proposeRegex');
    f.detectChanges();
    expect(root.textContent).toContain('50 lines');
    buttonByText(f, 'Suggest regex').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-proposal]'));
    expect(propose).toHaveBeenCalledWith(lines.slice(0, 50), 'the level');
    buttonByText(f, 'Use this pattern').click();
    expect(ref.close).toHaveBeenCalledWith(expect.stringContaining('(?<timestamp>'));
    buttonByText(f, 'Dismiss').click();
    f.detectChanges();
    expect(root.querySelector('[data-testid=ai-proposal]')).toBeNull();
  });

  it('offers the pattern as a highlight in highlight mode, and reports errors', async () => {
    const { f, root, backend, ref } = open(AiRegexDialog, { lines: ['one'], highlight: true });
    f.detectChanges();
    typeInto(el<HTMLInputElement>(f, '#ai-regex-intent'), '  ');
    vi.spyOn(backend, 'proposeRegex').mockRejectedValueOnce(new AiError('engine', 'nope'));
    buttonByText(f, 'Suggest regex').click();
    await until(f, () => !!root.querySelector('.err'));
    buttonByText(f, 'Suggest regex').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-proposal]'));
    vi.spyOn(TestBed.inject(ProposalApply), 'apply').mockResolvedValue({ kind: 'applied', message: 'ok' });
    buttonByText(f, 'Add as highlight…').click();
    await settle(f);
    expect(ref.close).toHaveBeenCalledWith();
    // A custom apply of another kind is ignored; empty lines do nothing.
    (f.componentInstance as unknown as { use(p: unknown): void }).use({ kind: 'query' });
    typeInto(el<HTMLTextAreaElement>(f, 'textarea'), '\n \n');
    f.detectChanges();
    await f.componentInstance.suggest();
    expect(root.textContent).toContain('0 lines');
  });
});

describe('AiProposeDialog', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('proposes each kind from a goal and the sample', async () => {
    for (const [kind, method] of [
      ['pipeline', 'proposePipeline'],
      ['markers', 'proposeMarkers'],
      ['extractors', 'proposeExtractors'],
    ] as const) {
      const { f, root, backend } = open(AiProposeDialog, { kind, source: { recordId: 9 }, initialGoal: 'timeouts' });
      const spy = vi.spyOn(backend, method);
      f.detectChanges();
      expect(root.textContent).toContain('the lines around event 9');
      buttonByText(f, 'Propose').click();
      await until(f, () => !!root.querySelector('[data-testid=ai-proposal]'));
      expect(spy).toHaveBeenCalledWith('timeouts', { recordId: 9 });
      vi.spyOn(TestBed.inject(ProposalApply), 'apply').mockResolvedValue({ kind: 'applied', message: 'saved' });
      buttonByText(f, 'Add to profile').click();
      await until(f, () => root.textContent!.includes('applied'));
      TestBed.resetTestingModule();
    }
  });

  it('proposes a profile without a goal, from given text', async () => {
    const { f, root, backend } = open(AiProposeDialog, { kind: 'profile', source: { sample: 'a\nb\nc' } });
    const spy = vi.spyOn(backend, 'proposeProfile');
    f.detectChanges();
    expect(root.textContent).toContain('3 lines you provided');
    expect(root.querySelector('textarea')).toBeNull();
    buttonByText(f, 'Propose').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-proposal]'));
    expect(spy).toHaveBeenCalledWith({ sample: 'a\nb\nc' });
    buttonByText(f, 'Dismiss').click();
    f.detectChanges();
    expect(root.querySelector('[data-testid=ai-proposal]')).toBeNull();
  });

  it('lets the user choose the kind and needs a goal', async () => {
    const { f, root, backend } = open(AiProposeDialog, { chooseKind: true });
    f.detectChanges();
    expect(root.textContent).toContain('Select an event first');
    expect(buttonByText(f, 'Propose').disabled).toBe(true);
    const select = el<HTMLSelectElement>(f, 'select');
    typeInto(select, 'markers', 'change');
    typeInto(select, 'bogus', 'change');
    f.detectChanges();
    expect(el(f, 'label[for=ai-propose-goal]').textContent).toContain('Which events should be marked?');
    typeInto(el<HTMLTextAreaElement>(f, 'textarea'), 'payment failures');
    f.detectChanges();
    vi.spyOn(backend, 'proposeMarkers').mockRejectedValueOnce(new AiError('invalid', 'select an event in the file first'));
    buttonByText(f, 'Propose').click();
    await until(f, () => !!root.querySelector('.err'));
    expect(el(f, '.err').textContent).toBe('select an event in the file first');
  });
});
