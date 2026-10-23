import { describe, expect, it } from 'vitest';
import { MOCK_HARDWARE, mockRecommendations } from '../../core/mock/mock-ai-data';
import { SetupPlan } from '../ai.types';
import { canStart, INITIAL, next, selected, WizardState } from './wizard';

function plan(over: Partial<SetupPlan> = {}): SetupPlan {
  const recommendations = mockRecommendations([]);
  return {
    hardware: MOCK_HARDWARE,
    recommendations,
    chosen: recommendations.best,
    license: 'apache-2.0',
    gated: false,
    downloadBytes: 1,
    alreadyInstalled: false,
    note: null,
    ...over,
  };
}

const run = (s: WizardState, ...events: Parameters<typeof next>[1][]) => events.reduce(next, s);

describe('setup wizard state machine', () => {
  it('loads a plan and preselects the best (or preferred) model', () => {
    const s = next(INITIAL, { type: 'planLoaded', plan: plan() });
    expect(s.kind).toBe('plan');
    expect(selected(s)?.model.id).toBe('qwen3.5-9b-q4km');
    expect(canStart(s)).toBe(true);
    const p = next(INITIAL, { type: 'planLoaded', plan: plan(), preferred: 'llama3.2-3b-q4km' });
    expect(selected(p)?.model.id).toBe('llama3.2-3b-q4km');
    const unknown = next(INITIAL, { type: 'planLoaded', plan: plan({ chosen: null }), preferred: 'nope' });
    expect(selected(unknown)?.model.id).toBe('qwen3.5-9b-q4km');
    const empty = next(INITIAL, { type: 'planLoaded', plan: plan({ chosen: null, recommendations: { ...plan().recommendations, ranked: [] } }) });
    expect(selected(empty)).toBeNull();
    expect(canStart(empty)).toBe(false);
    expect(selected(INITIAL)).toBeNull();
  });

  it('changes the selection only to a model of the plan', () => {
    const s = next(INITIAL, { type: 'planLoaded', plan: plan() });
    expect(selected(next(s, { type: 'select', modelId: 'llama3.2-3b-q4km' }))?.model.id).toBe('llama3.2-3b-q4km');
    expect(next(s, { type: 'select', modelId: 'nope' })).toBe(s);
    expect(next(INITIAL, { type: 'select', modelId: 'llama3.2-3b-q4km' })).toBe(INITIAL);
  });

  it('keeps the selection when the exact memory figures arrive, and ignores them before a plan', () => {
    const s = run(INITIAL, { type: 'planLoaded', plan: plan() }, { type: 'select', modelId: 'llama3.2-3b-q4km' });
    const refined = next(s, { type: 'fitRefined', recommendations: plan().recommendations });
    expect(selected(refined)?.model.id).toBe('llama3.2-3b-q4km');
    expect(next(INITIAL, { type: 'fitRefined', recommendations: plan().recommendations })).toBe(INITIAL);
  });

  it('cannot start a model that is too large for this computer', () => {
    const p = plan();
    const big = { ...p, recommendations: { ...p.recommendations, ranked: p.recommendations.ranked.map((r) => ({ ...r, assessment: { ...r.assessment, fit: 'tooLarge' as const } })) } };
    expect(canStart(next(INITIAL, { type: 'planLoaded', plan: big }))).toBe(false);
  });

  it('fails with the engine message and starts again from loading on retry', () => {
    const f = next(INITIAL, { type: 'failed', code: 'unavailable', message: 'no worker' });
    expect(f).toEqual({ kind: 'failed', code: 'unavailable', message: 'no worker' });
    expect(next(f, { type: 'retry' })).toBe(INITIAL);
    const p = next(INITIAL, { type: 'planLoaded', plan: plan() });
    expect(next(p, { type: 'retry' })).toBe(p);
  });
});
