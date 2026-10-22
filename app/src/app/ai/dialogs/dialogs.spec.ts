import { Provider, Type } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { MockAiBackend } from '../../core/mock/mock-ai-backend';
import { DIALOG_REF } from '../../ui/dialog.service';
import { all, buttonByText, el, settle, typeInto } from '../../../testing/helpers';
import { AiBackend, AiError } from '../ai-backend';
import { AiCitations } from '../chat/citation-actions';
import { Narrative, QueryProposal } from '../ai.types';
import { JobsBackend } from '../../jobs/jobs-backend';
import { JobOutcome } from '../../jobs/jobs.types';
import { ProposalApply } from '../proposals/proposal-apply';
import { AiNarrativeDialog } from './narrative-dialog';
import { AiProposeDialog } from './propose-dialog';
import { AiQueryDialog } from './query-dialog';
import { AiRegexDialog } from './regex-dialog';
import { at } from '../../../testing/must';

function open<T>(component: Type<T>, inputs: Record<string, unknown> = {}, extra: Provider[] = []) {
  const backend = new MockAiBackend({ stepMs: 0 });
  const ref = { close: vi.fn(), closed: new Promise(() => undefined) };
  TestBed.configureTestingModule({
    providers: [...provideAnalysisTesting({ context: new FakeContext() }), { provide: AiBackend, useValue: backend }, { provide: DIALOG_REF, useValue: ref }, ...extra],
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

/** A job backend whose jobs end when the test says so: what a dialog sees while its request runs in Jobs. */
function slowJobs() {
  let end!: (o: JobOutcome) => void;
  const jobs = {
    wait: vi.fn(() => new Promise<JobOutcome>((r) => (end = r))),
    unwait: vi.fn(async () => undefined),
    cancel: vi.fn(async () => undefined),
    list: vi.fn(async () => ({ jobs: [], paused: [] })),
    onChange: vi.fn(async () => () => undefined),
  };
  const outcome = (state: JobOutcome['state'], extra: Partial<JobOutcome> = {}): JobOutcome => ({ id: 41, state, result: null, error: null, message: null, ...extra });
  return { jobs, providers: [{ provide: JobsBackend, useValue: jobs }] as Provider[], end: (o: JobOutcome) => end(o), outcome };
}

const answer = (text: string, citations: Narrative['citations']): Narrative => ({ text, citations, evidence: { recordIds: [], queries: [] }, usage: { inputTokens: 0, outputTokens: 0 } });

describe('AiNarrativeDialog', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('shows a finished answer and follows its evidence', async () => {
    const preset = answer('Event 42 failed: see `level >= error`.', [
      { kind: 'record', id: 42, verified: true },
      { kind: 'query', ql: 'level >= error' },
    ]);
    const { f, ref, root } = open(AiNarrativeDialog, { preset });
    f.detectChanges();
    expect(el(f, '[data-testid=ai-narrative]').textContent).toContain('Event 42 failed');
    const cites = TestBed.inject(AiCitations);
    const record = vi.spyOn(cites, 'record').mockResolvedValue();
    const query = vi.spyOn(cites, 'query').mockResolvedValueOnce('bad').mockResolvedValueOnce(null);
    at(all<HTMLButtonElement>(f, 'button.cite.q'), 0).click();
    await until(f, () => !!root.querySelector('.note'));
    expect(el(f, '.note').textContent).toContain('Not applied: bad');
    at(all<HTMLButtonElement>(f, 'button.cite.q'), 0).click();
    await settle(f);
    expect(ref.close).toHaveBeenCalledTimes(1);
    at(all<HTMLButtonElement>(f, 'button.cite.rec'), 0).click();
    await settle(f);
    expect(record).toHaveBeenCalledWith(42);
    expect(query).toHaveBeenCalledTimes(2);
    buttonByText(f, 'Close').click();
  });

  it('marks a "not found" answer and opens web sources', async () => {
    const preset = answer('Not found in the logs. See https://w.example/a', [{ kind: 'web', title: 'W', url: 'https://w.example/a' }]);
    const { f, root } = open(AiNarrativeDialog, { preset });
    f.detectChanges();
    expect(root.querySelector('.answer.neutral')).toBeTruthy();
    vi.spyOn(TestBed.inject(AiCitations), 'web').mockResolvedValueOnce('refused').mockResolvedValueOnce(null);
    at(all<HTMLButtonElement>(f, 'button.cite.web'), 0).click();
    await until(f, () => !!root.querySelector('.note'));
    expect(el(f, '.note').textContent).toBe('refused');
    at(all<HTMLButtonElement>(f, 'button.cite.web'), 0).click();
    await settle(f);
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
    vi.spyOn(backend, 'start').mockRejectedValueOnce(new AiError('noModel', 'set up first'));
    typeInto(el<HTMLInputElement>(f, 'input'), 'errors');
    f.detectChanges();
    el<HTMLFormElement>(f, 'form').dispatchEvent(new Event('submit'));
    await until(f, () => !!root.querySelector('.err'));
    expect(el(f, '.err').textContent).toBe('set up first');
  });

  it('starts an AI job at once, shows the wait line with Cancel while it runs and the proposal when it is done', async () => {
    const slow = slowJobs();
    const { f, root, backend } = open(AiQueryDialog, {}, slow.providers);
    const start = vi.spyOn(backend, 'start').mockResolvedValue(41);
    f.detectChanges();
    typeInto(el<HTMLInputElement>(f, 'input'), 'errors in checkout');
    f.detectChanges();
    el<HTMLFormElement>(f, 'form').dispatchEvent(new Event('submit'));
    await until(f, () => !!root.querySelector('[data-testid=job-wait]'));
    expect(start).toHaveBeenCalledWith({ op: 'query', question: 'errors in checkout' });
    expect(slow.jobs.wait).toHaveBeenCalledWith(41);
    expect(root.textContent).toContain('The local model is writing the query');
    el<HTMLButtonElement>(f, '[data-testid=job-wait-cancel]').click();
    expect(slow.jobs.cancel).toHaveBeenCalledWith(41);
    const proposal: QueryProposal = await backend.nlToQuery('errors in checkout');
    slow.end(slow.outcome('done', { result: { type: 'proposal', proposal } }));
    await until(f, () => !!root.querySelector('[data-testid=ai-proposal]'));
    expect(root.querySelector('[data-testid=job-wait]')).toBeNull();
  });

  it('closing the dialog stops following the job (it goes on in Jobs); a cancelled job shows nothing', async () => {
    const slow = slowJobs();
    const { f, root, backend } = open(AiQueryDialog, { initial: 'warnings' }, slow.providers);
    vi.spyOn(backend, 'start').mockResolvedValue(41);
    f.detectChanges();
    await until(f, () => !!root.querySelector('[data-testid=job-wait]'));
    f.destroy();
    expect(slow.jobs.unwait).toHaveBeenCalledWith(41);

    TestBed.resetTestingModule();
    const second = slowJobs();
    const other = open(AiQueryDialog, { initial: 'warnings' }, second.providers);
    vi.spyOn(other.backend, 'start').mockResolvedValue(41);
    other.f.detectChanges();
    await until(other.f, () => !!other.root.querySelector('[data-testid=job-wait]'));
    second.end(second.outcome('cancelled'));
    await until(other.f, () => !other.root.querySelector('[data-testid=job-wait]'));
    expect(other.root.querySelector('.err')).toBeNull();
    expect(other.root.querySelector('[data-testid=ai-proposal]')).toBeNull();
  });

  it('shows why a job failed', async () => {
    const slow = slowJobs();
    const { f, root, backend } = open(AiQueryDialog, { initial: 'warnings' }, slow.providers);
    vi.spyOn(backend, 'start').mockResolvedValue(41);
    f.detectChanges();
    await until(f, () => !!root.querySelector('[data-testid=job-wait]'));
    slow.end(slow.outcome('failed', { error: 'the model ran out of memory' }));
    await until(f, () => !!root.querySelector('.err'));
    expect(el(f, '.err').textContent).toBe('the model ran out of memory');
  });

  it('waits for a question', async () => {
    const { f, backend } = open(AiQueryDialog);
    const ask = vi.spyOn(backend, 'start');
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
    const propose = vi.spyOn(backend, 'start');
    f.detectChanges();
    expect(root.textContent).toContain('50 lines');
    buttonByText(f, 'Suggest regex').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-proposal]'));
    expect(propose).toHaveBeenCalledWith({ op: 'regex', lines: lines.slice(0, 50), intent: 'the level' });
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
    vi.spyOn(backend, 'start').mockRejectedValueOnce(new AiError('engine', 'nope'));
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

describe('AiRegexDialog as a job', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('returns from start at once and follows the job with the wait line until the proposal arrives', async () => {
    const slow = slowJobs();
    const { f, root, backend } = open(AiRegexDialog, { lines: ['a 1', 'a 2'] }, slow.providers);
    const start = vi.spyOn(backend, 'start').mockResolvedValue(41);
    f.detectChanges();
    buttonByText(f, 'Suggest regex').click();
    await until(f, () => !!root.querySelector('[data-testid=job-wait]'));
    expect(start).toHaveBeenCalledWith({ op: 'regex', lines: ['a 1', 'a 2'], intent: null });
    expect(root.textContent).toContain('The local model suggests a pattern');
    slow.end(slow.outcome('done', { result: { type: 'proposal', proposal: await backend.proposeRegex(['a 1', 'a 2']) } }));
    await until(f, () => !!root.querySelector('[data-testid=ai-proposal]'));
    expect(root.querySelector('[data-testid=job-wait]')).toBeNull();
  });
});

describe('AiProposeDialog', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('proposes each kind from a goal and the sample', async () => {
    for (const kind of ['pipeline', 'markers', 'extractors'] as const) {
      const { f, root, backend } = open(AiProposeDialog, { kind, source: { recordId: 9 }, initialGoal: 'timeouts' });
      const spy = vi.spyOn(backend, 'start');
      f.detectChanges();
      expect(root.textContent).toContain('the lines around event 9');
      buttonByText(f, 'Propose').click();
      await until(f, () => !!root.querySelector('[data-testid=ai-proposal]'));
      expect(spy).toHaveBeenCalledWith({ op: kind, goal: 'timeouts', recordId: 9, sample: null });
      vi.spyOn(TestBed.inject(ProposalApply), 'apply').mockResolvedValue({ kind: 'applied', message: 'saved' });
      buttonByText(f, 'Add to profile').click();
      await until(f, () => root.textContent!.includes('applied'));
      TestBed.resetTestingModule();
    }
  });

  it('proposes a profile without a goal, from given text', async () => {
    const { f, root, backend } = open(AiProposeDialog, { kind: 'parser', source: { sample: 'a\nb\nc' } });
    const spy = vi.spyOn(backend, 'start');
    f.detectChanges();
    expect(root.textContent).toContain('3 lines you provided');
    expect(root.querySelector('textarea')).toBeNull();
    buttonByText(f, 'Propose').click();
    await until(f, () => !!root.querySelector('[data-testid=ai-proposal]'));
    expect(spy).toHaveBeenCalledWith({ op: 'parser', recordId: null, sample: 'a\nb\nc', fileName: null });
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
    vi.spyOn(backend, 'start').mockRejectedValueOnce(new AiError('invalid', 'select an event in the file first'));
    buttonByText(f, 'Propose').click();
    await until(f, () => !!root.querySelector('.err'));
    expect(el(f, '.err').textContent).toBe('select an event in the file first');
  });

  it('starts an AI job at once, shows the wait line, and the proposal when it is done; closing leaves the job running', async () => {
    const slow = slowJobs();
    const { f, root, backend } = open(AiProposeDialog, { kind: 'markers', source: { recordId: 9 }, initialGoal: 'timeouts' }, slow.providers);
    const start = vi.spyOn(backend, 'start').mockResolvedValue(41);
    f.detectChanges();
    buttonByText(f, 'Propose').click();
    await until(f, () => !!root.querySelector('[data-testid=job-wait]'));
    expect(start).toHaveBeenCalledWith({ op: 'markers', goal: 'timeouts', recordId: 9, sample: null });
    expect(root.textContent).toContain('The local model proposes');
    slow.end(slow.outcome('done', { result: { type: 'proposal', proposal: await backend.proposeMarkers('timeouts', { recordId: 9 }) } }));
    await until(f, () => !!root.querySelector('[data-testid=ai-proposal]'));
    expect(root.querySelector('[data-testid=job-wait]')).toBeNull();

    TestBed.resetTestingModule();
    const again = slowJobs();
    const other = open(AiProposeDialog, { kind: 'pipeline', source: { recordId: 9 }, initialGoal: 'noise' }, again.providers);
    vi.spyOn(other.backend, 'start').mockResolvedValue(41);
    other.f.detectChanges();
    buttonByText(other.f, 'Propose').click();
    await until(other.f, () => !!other.root.querySelector('[data-testid=job-wait]'));
    other.f.destroy();
    expect(again.jobs.unwait).toHaveBeenCalledWith(41);
  });
});
