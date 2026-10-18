import { describe, expect, it } from 'vitest';
import { MOCK_HARDWARE, mockRecommendations } from '../../core/mock/mock-ai-data';
import { SetupPlan } from '../ai.types';
import { canStart, INITIAL, next, selected, stageLabel, stageStep, WizardState } from './wizard';

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
    const unknown = next(INITIAL, {
      type: 'planLoaded',
      plan: plan({ chosen: null }),
      preferred: 'nope',
    });
    expect(selected(unknown)?.model.id).toBe('qwen3.5-9b-q4km');
    const empty = next(INITIAL, {
      type: 'planLoaded',
      plan: plan({ chosen: null, recommendations: { ...plan().recommendations, ranked: [] } }),
    });
    expect(selected(empty)).toBeNull();
    expect(canStart(empty)).toBe(false);
    expect(selected(INITIAL)).toBeNull();
  });

  it('asks for an inference runtime first when none is installed, keeping the preferred model', () => {
    const s = next(INITIAL, {
      type: 'planLoaded',
      plan: plan(),
      preferred: 'llama3.2-3b-q4km',
      needsRuntime: true,
    });
    expect(s).toMatchObject({ kind: 'runtime', preferred: 'llama3.2-3b-q4km' });
    expect(selected(s)).toBeNull();
    expect(next(s, { type: 'start' })).toBe(s);
    const p = next(s, { type: 'runtimeReady' });
    expect(p.kind).toBe('plan');
    expect(selected(p)?.model.id).toBe('llama3.2-3b-q4km');
    expect(next(p, { type: 'runtimeReady' })).toBe(p);
    expect(next(s, { type: 'setup', event: { stage: 'cancelled' } })).toEqual({
      kind: 'cancelled',
    });
  });

  it('selects only known models and refuses to start a model that is too large', () => {
    let s = next(INITIAL, { type: 'planLoaded', plan: plan() });
    expect(next(s, { type: 'select', modelId: 'nope' })).toBe(s);
    expect(next(INITIAL, { type: 'select', modelId: 'x' })).toBe(INITIAL);
    s = next(s, { type: 'select', modelId: 'qwen3.5-32b-q4km' });
    expect(canStart(s)).toBe(false);
    expect(next(s, { type: 'start' })).toBe(s);
    expect(next(INITIAL, { type: 'start' })).toBe(INITIAL);
  });

  it('runs through the stages to ready, keeping the CPU fallback note', () => {
    let s = run(INITIAL, { type: 'planLoaded', plan: plan() }, { type: 'start' });
    expect(s).toMatchObject({
      kind: 'running',
      modelId: 'qwen3.5-9b-q4km',
      stage: { stage: 'probing' },
    });
    s = next(s, {
      type: 'setup',
      event: { stage: 'downloading', bytes: 1, total: 2, bytesPerSec: 3, etaSecs: 4 },
    });
    expect(s.kind === 'running' && s.stage.stage).toBe('downloading');
    s = next(s, { type: 'setup', event: { stage: 'fellBackToCpu', reason: 'no GPU' } });
    expect(s.kind === 'running' && s.note).toBe('no GPU');
    s = next(s, { type: 'cancelRequested' });
    expect(s.kind === 'running' && s.cancelling).toBe(true);
    s = next(s, { type: 'setup', event: { stage: 'ready', model: 'm.gguf', backend: 'CPU' } });
    expect(s).toEqual({ kind: 'ready', model: 'm.gguf', backend: 'CPU', note: 'no GPU' });
    // Later stage events and cancel requests do not change a finished wizard.
    expect(next(s, { type: 'setup', event: { stage: 'warmingUp' } })).toBe(s);
    expect(next(s, { type: 'setup', event: { stage: 'fellBackToCpu', reason: 'x' } })).toBe(s);
    expect(next(s, { type: 'setup', event: { stage: 'cancelled' } })).toBe(s);
    expect(next(s, { type: 'cancelRequested' })).toBe(s);
    expect(next(s, { type: 'retry' })).toBe(s);
    expect(
      next(INITIAL, { type: 'setup', event: { stage: 'ready', model: 'a', backend: 'b' } }),
    ).toMatchObject({ kind: 'ready', note: null });
  });

  it('fails, cancels and retries', () => {
    const running = run(INITIAL, { type: 'planLoaded', plan: plan() }, { type: 'start' });
    const f = next(running, {
      type: 'setup',
      event: { stage: 'failed', code: 'network', message: 'offline' },
    });
    expect(f).toEqual({
      kind: 'failed',
      code: 'network',
      message: 'offline',
      modelId: 'qwen3.5-9b-q4km',
    });
    expect(next(f, { type: 'retry' })).toEqual(INITIAL);
    expect(next(INITIAL, { type: 'failed', code: 'engine', message: 'x' })).toMatchObject({
      modelId: null,
    });
    expect(next(running, { type: 'failed', code: 'busy', message: 'b' })).toMatchObject({
      modelId: 'qwen3.5-9b-q4km',
    });
    expect(
      next(INITIAL, { type: 'setup', event: { stage: 'failed', code: 'engine', message: 'x' } }),
    ).toMatchObject({ modelId: null });
    const c = next(running, { type: 'setup', event: { stage: 'cancelled' } });
    expect(c).toEqual({ kind: 'cancelled' });
    expect(next(INITIAL, { type: 'setup', event: { stage: 'cancelled' } })).toEqual({
      kind: 'cancelled',
    });
    expect(next(c, { type: 'retry' })).toEqual(INITIAL);
  });

  it('names the stages', () => {
    const cases: [Parameters<typeof stageLabel>[0], number, string][] = [
      [{ stage: 'probing' }, 1, 'Checking this computer'],
      [{ stage: 'recommending' }, 1, 'Choosing'],
      [
        { stage: 'downloading', bytes: 0, total: 1, bytesPerSec: 0, etaSecs: null },
        2,
        'Downloading',
      ],
      [{ stage: 'verifying', bytes: 0, total: 1 }, 3, 'Verifying'],
      [{ stage: 'validating' }, 3, 'Checking the model'],
      [{ stage: 'loading', gpu: true }, 4, 'GPU'],
      [{ stage: 'loading', gpu: false }, 4, 'CPU'],
      [{ stage: 'warmingUp' }, 5, 'Warming'],
    ];
    for (const [stage, step, text] of cases) {
      expect(stageStep(stage)).toBe(step);
      expect(stageLabel(stage)).toContain(text);
    }
  });
});
