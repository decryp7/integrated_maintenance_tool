import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { QuerySession } from '../../analysis/query-session';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { MarkDialog } from '../../context/mark-dialog';
import { mockExtractorsProposal, mockMarkersProposal, mockPipelineProposal, mockParserProposal, mockRegexProposal } from '../../core/mock/mock-ai-data';
import { MockAiBackend } from '../../core/mock/mock-ai-backend';
import { ProfileUndo } from '../../undo/profile-undo';
import { WorkspaceStore } from '../../core/workspace.store';
import { ParsersBackend } from '../../settings/parsers/parsers-backend';
import { DialogRef, DialogService } from '../../ui/dialog.service';
import { all, buttonByText, settle } from '../../../testing/helpers';
import { AiBackend } from '../ai-backend';
import { Proposal, QueryProposal } from '../ai.types';
import { ProposalApply } from './proposal-apply';
import { AiProposalCard } from './proposal-card';
import { at } from '../../../testing/must';

const query: QueryProposal = { kind: 'query', ql: 'level >= error', explanation: 'Errors.', estimatedMatches: 1, repaired: true };

function configure() {
  const ctx = new FakeContext();
  TestBed.configureTestingModule({ providers: [...provideAnalysisTesting({ context: ctx }), { provide: AiBackend, useValue: new MockAiBackend({ stepMs: 0 }) }] });
  return ctx;
}

function fakeRef<R>(result: R | undefined): DialogRef<R> {
  return { close: vi.fn(), closed: Promise.resolve(result) };
}

describe('ProposalApply', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('offers a regex as a highlight rule in the marker dialog', async () => {
    configure();
    const dialogs = TestBed.inject(DialogService);
    const open = vi.spyOn(dialogs, 'open').mockReturnValueOnce(fakeRef(true)).mockReturnValueOnce(fakeRef(undefined));
    const apply = TestBed.inject(ProposalApply);
    const p = mockRegexProposal(['a']);
    expect(await apply.apply(p)).toEqual({ kind: 'applied', message: 'Highlight rule added.' });
    expect(at(at(open.mock.calls, 0), 0)).toBe(MarkDialog);
    expect(at(open.mock.calls, 0)[1]?.inputs).toMatchObject({ initial: [{ field: '@message', op: 'matches', value: p.pattern }], submitLabel: 'Add highlight' });
    expect((at(open.mock.calls, 0)[1]?.inputs as { suggestedLabel: string }).suggestedLabel.endsWith('…')).toBe(true);
    expect(await apply.apply({ ...p, rationale: ' ' })).toEqual({ kind: 'cancelled' });
    expect(at(open.mock.calls, 1)[1]?.inputs).toMatchObject({ suggestedLabel: 'AI pattern' });
  });

  it('applies a query through the query bar', async () => {
    const ctx = configure();
    const session = TestBed.inject(QuerySession);
    vi.spyOn(session, 'run').mockResolvedValueOnce('events').mockResolvedValueOnce('table').mockResolvedValueOnce(null).mockResolvedValueOnce(null);
    const apply = TestBed.inject(ProposalApply);
    expect(await apply.apply(query)).toEqual({ kind: 'applied', message: 'Query applied as the filter.' });
    expect(session.text()).toBe('level >= error');
    expect((await apply.apply(query)).kind).toBe('applied');
    expect(ctx.last('showView')).toEqual(['query-results']);
    await expect(apply.apply(query)).rejects.toThrow('The query could not be run.');
    vi.spyOn(session, 'runError').mockReturnValue('bad');
    await expect(apply.apply(query)).rejects.toThrow('bad');
  });

  it('saves profile changes through the validated, undoable profile save', async () => {
    configure();
    const save = vi.spyOn(TestBed.inject(ProfileUndo), 'save').mockResolvedValue(null as never);
    const apply = TestBed.inject(ProposalApply);
    const pipeline = mockPipelineProposal('g');
    const markers = mockMarkersProposal('g');
    const extractors = mockExtractorsProposal('g');
    for (const p of [pipeline, markers, extractors]) expect((await apply.apply(p)).kind).toBe('applied');
    expect(save.mock.calls).toEqual([
      ['AI: pipeline steps', pipeline.profileToml],
      ['AI: markers', markers.profileToml],
      ['AI: extractors', extractors.profileToml],
    ]);
  });

  it('adds a proposed parser to the library and reads the files again, or says the library reads the file', async () => {
    configure();
    const parsers = TestBed.inject(ParsersBackend);
    const saveParser = vi.spyOn(parsers, 'save').mockResolvedValue({ parsers: [], broken: [], stale: 1 });
    const reread = vi.spyOn(parsers, 'reread').mockResolvedValue(7);
    const follow = vi.spyOn(TestBed.inject(WorkspaceStore), 'followReread').mockResolvedValue();
    const apply = TestBed.inject(ProposalApply);
    const profile = mockParserProposal();
    expect(profile.toml).toContain('schema = 1');
    expect(await apply.apply(profile)).toEqual({ kind: 'applied', message: 'Parser added to the library; the files are read again.' });
    expect(saveParser).toHaveBeenCalledWith(profile.toml, { overwrite: true });
    expect(reread).toHaveBeenCalled();
    expect(follow).toHaveBeenCalledWith(7);
    expect(await apply.apply({ ...profile, toml: '' })).toEqual({ kind: 'applied', message: "Debrief's parsers already read this file." });
    expect(saveParser).toHaveBeenCalledTimes(1);
  });
});

@Component({
  imports: [AiProposalCard],
  template: `<debrief-ai-proposal-card [proposal]="p()" [state]="state()" [customApply]="custom()" [applyLabel]="label()" (applied)="events.push('applied:' + $event)" (applyRequested)="events.push('requested:' + $event.kind)" (dismissed)="events.push('dismissed')" />`,
})
class CardHost {
  readonly p = signal<Proposal>(query);
  readonly state = signal<'open' | 'applied' | 'dismissed'>('open');
  readonly custom = signal(false);
  readonly label = signal('');
  readonly events: string[] = [];
}

describe('AiProposalCard', () => {
  afterEach(() => TestBed.resetTestingModule());

  function card() {
    configure();
    const f = TestBed.createComponent(CardHost);
    f.detectChanges();
    return f;
  }

  it('shows the engine evidence of every kind', () => {
    const f = card();
    const root = f.nativeElement as HTMLElement;
    const cases: [Proposal, string[]][] = [
      [query, ['Query', 'level >= error', '1 result now', 'repaired after a syntax error', 'Apply query']],
      [{ ...query, estimatedMatches: null, repaired: false }, ['Errors.']],
      [mockRegexProposal(['2026-10-14 09:00:02 [INF] x', 'nope']), ['Regular expression', 'Matches 1 of 2 examples', 'timestamp: timestamp', 'Group "level"', 'Add as highlight…']],
      [{ ...mockRegexProposal(['x']), source: 'inferred', groups: [], explanation: [] }, ["the engine's own inference", 'Matches 0 of 1 example']],
      [mockParserProposal(), ['Parser', 'Worker text logs · text · parses 58 entries, 58 with a timestamp', 'Queue depth 812', 'Parser TOML', 'Add to parsers']],
      [{ ...mockParserProposal(), toml: '' }, ["Debrief's parsers already read this file."]],
      [{ ...mockParserProposal(), importer: null, preview: [] }, ['no importer']],
      [mockPipelineProposal('g'), ['Pipeline steps', 'keeps 52, drops 8, changes 0', '60 → 52', 'GET /health check', 'Add to profile']],
      [mockMarkersProposal('g'), ['Markers', 'Timeout', '6 matches']],
      [{ ...mockMarkersProposal('g'), markers: [{ kind: 'k', label: 'K', matches: 1 }] }, ['1 match']],
      [mockExtractorsProposal('g'), ['Extractors', 'elapsed', '31 matches', 'e.g. elapsed=120']],
      [{ ...mockExtractorsProposal('g'), extractors: [{ name: 'n', fields: ['a'], matches: 1, examples: [{ s: 'v' }] }] }, ['1 match', 'e.g. s=v']],
    ];
    for (const [p, texts] of cases) {
      f.componentInstance.p.set(p);
      f.detectChanges();
      for (const t of texts) expect(root.textContent, p.kind).toContain(t);
    }
  });

  it('applies through ProposalApply and reports the outcome', async () => {
    const f = card();
    const apply = vi.spyOn(TestBed.inject(ProposalApply), 'apply');
    apply.mockResolvedValueOnce({ kind: 'applied', message: 'done' });
    buttonByText(f, 'Apply query').click();
    f.detectChanges();
    expect(buttonByText(f, 'Applying…').disabled).toBe(true);
    await settle(f);
    f.detectChanges();
    expect(f.componentInstance.events).toEqual(['applied:done']);
    expect((f.nativeElement as HTMLElement).querySelector('[role=status]')?.textContent).toContain('done');
    apply.mockResolvedValueOnce({ kind: 'cancelled' });
    buttonByText(f, 'Apply query').click();
    await settle(f);
    apply.mockRejectedValueOnce(new Error('profile invalid'));
    buttonByText(f, 'Apply query').click();
    await settle(f);
    f.detectChanges();
    expect(f.componentInstance.events).toEqual(['applied:done']);
    expect((f.nativeElement as HTMLElement).querySelector('[role=alert]')?.textContent).toContain('Not applied: profile invalid');
    buttonByText(f, 'Dismiss').click();
    expect(f.componentInstance.events.at(-1)).toBe('dismissed');
  });

  it('lets the host apply, and hides actions once decided', () => {
    const f = card();
    f.componentInstance.custom.set(true);
    f.componentInstance.label.set('Use this pattern');
    f.detectChanges();
    buttonByText(f, 'Use this pattern').click();
    expect(f.componentInstance.events).toEqual(['requested:query']);
    f.componentInstance.state.set('applied');
    f.detectChanges();
    expect(all(f, 'button')).toHaveLength(0);
    expect((f.nativeElement as HTMLElement).textContent).toContain('applied');
    f.componentInstance.state.set('dismissed');
    f.detectChanges();
    expect((f.nativeElement as HTMLElement).textContent).toContain('dismissed');
  });
});
