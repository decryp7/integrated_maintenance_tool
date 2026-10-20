import { ExceptionsView } from './exceptions-view';
import { TestBed } from '@angular/core/testing';
import { settle, text } from '../../testing/helpers';
import { AnalysisCoverage } from '../core/api.types';
import { TauriAnalysisBackend } from '../core/analysis-backend';
import { MockAnalysisBackend } from './mock/mock-analysis-backend';
import { EMPTY_QUERY } from '../core/workspace.store';
import { loadWorkspace, provideAnalysisTesting, stubBackend } from './test-setup';

async function mount(coverage: () => Promise<AnalysisCoverage | null>) {
  TestBed.configureTestingModule({ providers: provideAnalysisTesting({ backend: stubBackend({ analysisCoverage: coverage }) }) });
  await loadWorkspace();
  const fixture = TestBed.createComponent(ExceptionsView);
  fixture.detectChanges();
  await settle(fixture, 6);
  return fixture;
}

describe('CoverageNote', () => {
  it('says when only part of the selection was analysed', async () => {
    const fixture = await mount(async () => ({ analysed: 2000000, total: 2500000 }));
    expect(text(fixture)).toContain('Analysed the first 2,000,000 of 2,500,000 matching events');
  });

  it('stays hidden when everything was analysed, the engine cannot say, or it fails', async () => {
    for (const answer of [async () => ({ analysed: 5, total: 5 }), async () => null, () => Promise.reject(new Error('down'))]) {
      TestBed.resetTestingModule();
      const fixture = await mount(answer);
      expect(text(fixture)).not.toContain('Analysed the first');
    }
  });

  it('asks the engine through analysis_coverage, and the mock covers everything', async () => {
    const call = vi.fn().mockResolvedValue({ analysed: 1, total: 1 });
    expect(await new TauriAnalysisBackend(call as never).analysisCoverage(EMPTY_QUERY)).toEqual({ analysed: 1, total: 1 });
    expect(call).toHaveBeenCalledWith('analysis_coverage', { request: { query: EMPTY_QUERY } });
    expect(await new MockAnalysisBackend(async () => []).analysisCoverage(EMPTY_QUERY)).toEqual({ analysed: 0, total: 0 });
  });
});
