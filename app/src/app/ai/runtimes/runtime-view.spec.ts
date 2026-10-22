import { describe, expect, it } from 'vitest';
import { MockRuntimes } from '../../core/mock/mock-ai-runtimes';
import { AiRuntimeEntry, AiRuntimeOverview } from '../ai.types';
import {
  deviceLine,
  entryState,
  entryStateLabel,
  isTerminal,
  orderedBuilds,
  runtimePercent,
  runtimeProgressText,
  runtimeStageLabel,
  suggestedDownload,
} from './runtime-view';

const entry = (over: Partial<AiRuntimeEntry> = {}): AiRuntimeEntry => ({
  id: 'vulkan',
  label: 'Vulkan',
  downloadBytes: 1,
  installed: false,
  installedBytes: null,
  usable: null,
  problem: null,
  devices: [],
  suggested: false,
  active: false,
  ...over,
});

const overview = (mode: 'installed' | 'none' | 'broken'): AiRuntimeOverview =>
  new MockRuntimes(mode, 0, null).overview();

describe('runtime view helpers', () => {
  it('names the install stages and their progress', () => {
    expect(
      runtimeStageLabel(
        { stage: 'downloading', id: 'cuda-13', asset: 'a.zip', index: 2, count: 2 },
        'CUDA 13',
      ),
    ).toBe('Downloading CUDA 13 (file 2 of 2)…');
    expect(
      runtimeStageLabel(
        { stage: 'downloading', id: 'cpu', asset: 'a.zip', index: 1, count: 1 },
        'CPU',
      ),
    ).toBe('Downloading CPU…');
    const tick = {
      stage: 'progress' as const,
      id: 'cpu',
      bytes: 10 * 1024 * 1024,
      total: 20 * 1024 * 1024,
      bytesPerSec: 5 * 1024 * 1024,
      etaSecs: 2,
      verifying: false,
    };
    expect(runtimeStageLabel(tick, 'CPU')).toBe('Downloading CPU…');
    expect(runtimeStageLabel({ ...tick, verifying: true }, 'CPU')).toBe('Verifying CPU…');
    expect(runtimeStageLabel({ stage: 'unpacking', id: 'cpu', asset: 'a.zip' }, 'CPU')).toBe(
      'Unpacking CPU…',
    );
    expect(runtimeStageLabel({ stage: 'finishing', id: 'cpu' }, 'CPU')).toBe('Finishing CPU…');
    expect(runtimeStageLabel({ stage: 'testing', id: 'cpu' }, 'CPU')).toBe('Testing CPU with your model…');
    expect(runtimePercent(tick)).toBe(50);
    expect(runtimePercent({ stage: 'finishing', id: 'cpu' })).toBeNull();
    expect(runtimePercent(null)).toBeNull();
    expect(runtimeProgressText(tick)).toBe('10 MB of 20 MB · 5 MB/s · less than a minute left');
    expect(runtimeProgressText({ ...tick, verifying: true })).toBe('10 MB of 20 MB');
    expect(runtimeProgressText({ ...tick, bytesPerSec: 0, etaSecs: null })).toBe('10 MB of 20 MB');
    expect(runtimeProgressText(null)).toBe('');
    expect(isTerminal({ stage: 'cancelled', id: 'x' })).toBe(true);
    expect(isTerminal({ stage: 'finishing', id: 'x' })).toBe(false);
  });

  it('describes where a build stands and its devices', () => {
    expect([entryState(entry()), entryStateLabel(entry())]).toEqual(['available', 'Not installed']);
    expect(entryState(entry({ installed: true, active: true, usable: true }))).toBe('active');
    expect(entryStateLabel(entry({ installed: true, active: true }))).toBe('In use');
    expect(entryStateLabel(entry({ installed: true, usable: true }))).toBe('Installed, works here');
    expect(entryStateLabel(entry({ installed: true, usable: false }))).toBe(
      'Installed, does not work here',
    );
    expect(entryStateLabel(entry({ installed: true }))).toBe('Installed');
    expect(
      deviceLine({ name: 'CUDA0', description: 'RTX 4060', totalMib: 8192, freeMib: 7577 }),
    ).toBe('RTX 4060 (CUDA0, 7.4 GB of 8 GB free)');
  });

  it('offers the suggested build first while nothing usable is installed', () => {
    expect(suggestedDownload(overview('none'))?.id).toBe('cuda-13');
    expect(suggestedDownload(overview('broken'))?.id).toBe('cuda-13');
    expect(suggestedDownload(overview('installed'))).toBeNull();
    expect(suggestedDownload({ ...overview('none'), customPath: '/x/llama-server' })).toBeNull();
    const o = overview('none');
    expect(
      suggestedDownload({ ...o, builds: o.builds.map((b) => ({ ...b, suggested: false })) }),
    ).toBeNull();
    expect(suggestedDownload(null)).toBeNull();
    expect(
      orderedBuilds(overview('installed'))
        .map((b) => b.id)
        .slice(0, 3),
    ).toEqual(['cuda-13', 'vulkan', 'cpu']);
    expect(orderedBuilds(null)).toEqual([]);
  });
});
