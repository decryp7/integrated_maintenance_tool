import { TestBed } from '@angular/core/testing';
import { settle, text } from '../../testing/helpers';
import { Coverage, fullCoverage, isPartial } from '../core/api.types';
import { TauriAnalysisBackend } from '../core/analysis-backend';
import { EMPTY_QUERY } from '../core/workspace.store';
import { AnalysisResource } from './async-resource';
import { CoverageNote } from './coverage-note';
import { ExceptionsView } from './exceptions-view';
import { MockAnalysisBackend } from './mock/mock-analysis-backend';
import { FIX, loadWorkspace, provideAnalysisTesting, stubBackend } from './test-setup';

async function mount(coverage: Coverage) {
  TestBed.configureTestingModule({ providers: provideAnalysisTesting({ backend: stubBackend({ queryExceptions: async () => ({ result: FIX.exceptions, coverage }) }) }) });
  await loadWorkspace();
  const fixture = TestBed.createComponent(ExceptionsView);
  fixture.detectChanges();
  await settle(fixture, 6);
  return fixture;
}

describe('coverage', () => {
  it('is partial when fewer events were analysed than matched, or a limit bounded the result', () => {
    expect(isPartial(fullCoverage(5))).toBe(false);
    expect(isPartial({ matched: 5, analysed: 4, limits: [] })).toBe(true);
    expect(isPartial({ matched: 5, analysed: 5, limits: ['patterns: 10,000 kept'] })).toBe(true);
    expect(fullCoverage(7)).toEqual({ matched: 7, analysed: 7, limits: [] });
  });

  it('comes with the analysis reply: the resource exposes result and coverage, latest call wins', async () => {
    const r = new AnalysisResource<number[]>();
    expect(r.result()).toBeNull();
    expect(r.coverage()).toBeNull();
    await r.load(async () => ({ result: [1, 2], coverage: { matched: 9, analysed: 2, limits: [] } }));
    expect(r.result()).toEqual([1, 2]);
    expect(r.coverage()).toEqual({ matched: 9, analysed: 2, limits: [] });
    r.reset();
    expect(r.coverage()).toBeNull();
  });
});

describe('CoverageNote', () => {
  it('says when only part of the selection was analysed', async () => {
    const fixture = await mount({ matched: 2_500_000, analysed: 2_000_000, limits: [] });
    expect(text(fixture)).toContain('Analysed the first 2,000,000 of 2,500,000 matching events');
  });

  it('lists the limits that bounded the result', async () => {
    const fixture = await mount({ matched: 10, analysed: 10, limits: ['exceptions: 500 groups kept', 'stack traces cut at 64 frames'] });
    expect(text(fixture)).toContain('The result was limited');
    const items = [...(fixture.nativeElement as HTMLElement).querySelectorAll('[role=note] li')].map((li) => li.textContent?.trim());
    expect(items).toEqual(['exceptions: 500 groups kept', 'stack traces cut at 64 frames']);
  });

  it('stays hidden when everything was analysed or there is no reply yet', async () => {
    const fixture = await mount(fullCoverage(5));
    expect((fixture.nativeElement as HTMLElement).querySelector('[role=note]')).toBeNull();
    TestBed.resetTestingModule();
    const bare = TestBed.createComponent(CoverageNote);
    bare.detectChanges();
    expect(text(bare)).toBe('');
  });

  it('is read from the analysis reply (no analysis_coverage command), and the mock covers everything', async () => {
    const reply = { result: [], coverage: { matched: 1, analysed: 1, limits: [] } };
    const call = vi.fn().mockResolvedValue(reply);
    expect(await new TauriAnalysisBackend(call as never).queryExceptions(EMPTY_QUERY)).toEqual(reply);
    expect(call).toHaveBeenCalledWith('query_exceptions', { request: { query: EMPTY_QUERY } });
    expect((await new MockAnalysisBackend(async () => []).queryExceptions(EMPTY_QUERY)).coverage).toEqual(fullCoverage(0));
  });
});
