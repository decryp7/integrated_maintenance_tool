import { TestBed } from '@angular/core/testing';
import { signal } from '@angular/core';
import { describe, expect, it } from 'vitest';
import { runtimeProgressText } from '../../ai/runtimes/runtime-view';
import { progressText } from '../../updates/updates-dialog';
import { transferSpeed } from './transfer-speed';

const MB = 1024 * 1024;

describe('transferSpeed', () => {
  it('hides speed until 2 s of samples, then smooths, and resets when the transfer ends', () => {
    let t = 0;
    const progress = signal<{ bytes: number; total: number } | null>({ bytes: 0, total: 100 * MB });
    const speed = TestBed.runInInjectionContext(() => transferSpeed(progress, () => t));
    TestBed.tick();
    expect(speed()).toEqual({ rate: 0, etaSecs: null });
    for (const s of [1, 2, 3, 4]) {
      t = s * 1000;
      progress.set({ bytes: s * 10 * MB, total: 100 * MB });
      TestBed.tick();
    }
    expect(speed().rate).toBeCloseTo(10 * MB, -3);
    expect(speed().etaSecs).toBeCloseTo(6, 1);
    progress.set(null);
    TestBed.tick();
    expect(speed()).toEqual({ rate: 0, etaSecs: null });
  });

  it('feeds the runtime and update progress lines', () => {
    const speed = { rate: 12 * MB, etaSecs: 150 };
    const tick = { stage: 'progress' as const, id: 'cpu', bytes: 10 * MB, total: 20 * MB, bytesPerSec: 0, etaSecs: null, verifying: false };
    expect(runtimeProgressText(tick, speed)).toBe('10 MB of 20 MB · 12 MB/s · about 3 min left');
    expect(progressText(5 * MB, 50 * MB, speed)).toBe('5 MB of 50 MB · 12 MB/s · about 3 min left');
    expect(progressText(5 * MB, 50 * MB)).toBe('5 MB of 50 MB');
    expect(progressText(5 * MB, null)).toBe('5 MB');
  });
});
