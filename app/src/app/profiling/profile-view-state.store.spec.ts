import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { Evidence, FrameDto } from '../core/api.types';
import { cleanPatterns, DEFAULT_FRAMEWORK_PATTERNS, MAX_PATTERN_LEN, MAX_PATTERNS, ProfileViewState } from './profile-view-state.store';

const frame: FrameDto = { id: 3, name: 'A.B.C', module: 'A', file: null, line: null, kind: 'managed', isFramework: false };

function evidence(patch: Partial<Evidence> = {}): Evidence {
  return { view: 'callTree', profileId: 2, baselineId: null, frame, path: [0, 3], metric: 'timeNs', range: null, threads: null, eventKind: null, ...patch };
}

describe('ProfileViewState', () => {
  const store = () => TestBed.inject(ProfileViewState);

  it('builds the engine view from range, threads, just my code and recursion', () => {
    const s = store();
    expect(s.view()).toEqual({ range: null, threads: null, justMyCode: null, recursion: 'off' });
    expect(s.filtered()).toBe(false);
    s.setRange({ fromNs: 10.4, toNs: 99.2 });
    s.setThreads([3, 1]);
    s.setJustMyCode(true);
    s.setRecursion('all');
    expect(s.view()).toEqual({
      range: { fromNs: 10, toNs: 100 },
      threads: [1, 3],
      justMyCode: { enabled: true, frameworkPatterns: DEFAULT_FRAMEWORK_PATTERNS, useFrameFlags: true },
      recursion: 'all',
    });
    expect(s.filtered()).toBe(true);
  });

  it('swaps a reversed range, clamps at zero and clears', () => {
    const s = store();
    s.setRange({ fromNs: 50, toNs: -5 });
    expect(s.range()).toEqual({ fromNs: 0, toNs: 50 });
    s.setRange(null);
    expect(s.range()).toBeNull();
  });

  it('toggles threads and drops an empty selection', () => {
    const s = store();
    s.toggleThread(2);
    s.toggleThread(1);
    expect(s.threads()).toEqual([1, 2]);
    s.toggleThread(2);
    s.toggleThread(1);
    expect(s.threads()).toBeNull();
    s.setThreads([]);
    expect(s.threads()).toBeNull();
    s.setThreads(null);
    expect(s.threads()).toBeNull();
  });

  it('drops per-profile selection when another profile becomes active', () => {
    const s = store();
    s.setProfile(1);
    s.setBaseline(2);
    s.setRange({ fromNs: 1, toNs: 2 });
    s.selectFrame(frame);
    s.setFlameFocus([1, 2]);
    s.setFlameSearch('x');
    s.setMetric('samples');
    s.setProfile(1);
    expect(s.range()).not.toBeNull();
    s.setProfile(2);
    expect(s.range()).toBeNull();
    expect(s.selectedFrame()).toBeNull();
    expect(s.flameFocus()).toEqual([]);
    expect(s.flameSearch()).toBe('');
    expect(s.metric()).toBeNull();
    // The baseline cannot be the target.
    expect(s.baselineId()).toBeNull();
    s.setBaseline(1);
    s.setProfile(3);
    expect(s.baselineId()).toBe(1);
  });

  it('applies evidence: profile, baseline, range, threads, frame, reveal and flame focus', () => {
    const s = store();
    s.setBaseline(9);
    s.applyEvidence(evidence({ range: { fromNs: 1, toNs: 5 }, threads: [2], eventKind: 'gc' }));
    expect(s.profileId()).toBe(2);
    expect(s.baselineId()).toBe(9);
    expect(s.range()).toEqual({ fromNs: 1, toNs: 5 });
    expect(s.threads()).toEqual([2]);
    expect(s.selectedFrame()).toEqual(frame);
    expect(s.eventKind()).toBe('gc');
    expect(s.reveal()).toEqual({ path: [0, 3], direction: 'topDown', nonce: 1 });
    expect(s.flameFocus()).toEqual([]);
    s.applyEvidence(evidence({ view: 'flame', baselineId: 4, threads: [], path: [] }));
    expect(s.flameFocus()).toEqual([]);
    expect(s.baselineId()).toBe(4);
    expect(s.threads()).toBeNull();
    expect(s.reveal()).toBeNull();
    s.applyEvidence(evidence({ view: 'flame' }));
    expect(s.flameFocus()).toEqual([0, 3]);
  });

  it('counts reveals and capture changes, sets the remaining fields and resets', () => {
    const s = store();
    s.revealPath([1]);
    s.revealPath([2], 'bottomUp');
    expect(s.reveal()).toEqual({ path: [2], direction: 'bottomUp', nonce: 2 });
    s.capturesChanged();
    expect(s.capturesVersion()).toBe(1);
    s.setHeap(7);
    s.setUseFrameFlags(false);
    s.setEventKind('jit');
    s.setFrameworkPatterns([' a* ', '', 'a*', 'b']);
    expect(s.frameworkPatterns()).toEqual(['a*', 'b']);
    expect(s.heapId()).toBe(7);
    expect(s.useFrameFlags()).toBe(false);
    expect(s.eventKind()).toBe('jit');
    s.reset();
    expect(s.heapId()).toBeNull();
    expect(s.capturesVersion()).toBe(0);
  });
});

describe('cleanPatterns', () => {
  it('trims, drops empty, duplicate and over-long patterns, and caps the count', () => {
    expect(cleanPatterns([' x ', 'x', '', 'y'.repeat(MAX_PATTERN_LEN + 1)])).toEqual(['x']);
    const many = Array.from({ length: MAX_PATTERNS + 5 }, (_, i) => `p${i}`);
    expect(cleanPatterns(many)).toHaveLength(MAX_PATTERNS);
  });
});
