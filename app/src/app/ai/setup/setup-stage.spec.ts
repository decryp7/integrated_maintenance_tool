import { describe, expect, it } from 'vitest';
import { RunningStage, stageLabel } from './setup-stage';

describe('stageLabel', () => {
  it('names every running stage in plain words', () => {
    const stages: RunningStage[] = [
      { stage: 'probing' },
      { stage: 'recommending' },
      { stage: 'downloading', bytes: 1, total: 2, bytesPerSec: 1, etaSecs: 1, connections: 1 },
      { stage: 'verifying', bytes: 1, total: 2 },
      { stage: 'validating' },
      { stage: 'loading', gpu: true },
      { stage: 'loading', gpu: false },
      { stage: 'warmingUp' },
    ];
    const labels = stages.map(stageLabel);
    expect(new Set(labels).size).toBe(labels.length);
    expect(labels[5]).toContain('GPU');
    expect(labels[6]).toContain('CPU');
  });
});
