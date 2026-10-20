import { ApplicationRef, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle } from '../../testing/helpers';
import { Lane, MarkerKind, Timeline } from '../core/api.types';
import { stableLanes } from './lane-order';
import { buildLaneVms, densityPath, laneTitle } from './timeline-lane-vm';
import { TimelinePanel } from './timeline-panel';

const lane = (key: string, first: number, patch: Partial<Lane> = {}): Lane => ({ key, first, last: first + 10, total: 4, matched: 4, density: [], markers: [], sources: [], ...patch });
const timeline = (lanes: Lane[]): Timeline => ({ from: 0, to: 100, bucketMs: 10, histogram: [], lanes, truncatedLanes: 0 }) as unknown as Timeline;

describe('stableLanes', () => {
  it('keeps the unzoomed order while a range is shown and takes the new order once it is cleared', () => {
    const t = signal<Timeline | null>(timeline([lane('a', 1), lane('b', 2)]));
    const zoomed = signal(false);
    const lanes = TestBed.runInInjectionContext(() => stableLanes(t, zoomed));
    expect(lanes().map((l) => l.key)).toEqual(['a', 'b']);
    zoomed.set(true);
    t.set(timeline([lane('b', 0), lane('c', -5), lane('a', 3)]));
    expect(lanes().map((l) => l.key)).toEqual(['a', 'b', 'c']);
    zoomed.set(false);
    expect(lanes().map((l) => l.key)).toEqual(['b', 'c', 'a']);
    t.set(null);
    expect(lanes()).toEqual([]);
  });
});

describe('buildLaneVms', () => {
  const kinds = new Map<string, MarkerKind>([['fatal', { kind: 'fatal', label: 'Fatal', color: '#f00' } as MarkerKind]]);
  const base = {
    kinds,
    groups: new Set(['b']),
    timezone: 'utc' as const,
    plotWidth: 1000,
    main: true,
    marks: () => [],
    moreSources: (l: Lane) => Math.max(0, l.sources.length - 2),
    markerAria: (m: { label: string }) => `aria ${m.label}`,
  };

  it('lays out extent, counts, titles, marker targets and source overflow', () => {
    const lanes = [
      lane('a', 10, { matched: 2, markers: [{ kind: 'fatal', bucket: 5, ts: 50, id: 7, count: 1, message: 'x' }], sources: [1, 2, 3] }),
      lane('b', 20),
    ];
    const [a, b] = buildLaneVms(lanes, timeline(lanes), base);
    expect(a!.left).toBe(10);
    expect(a!.width).toBe(10);
    expect(a!.meta).toMatch(/^2 of 4 · /);
    expect(a!.filtered).toBe(false);
    expect(a!.title).toBe(laneTitle('a', false, true));
    expect(a!.moreSources).toBe(1);
    expect(a!.targets).toHaveLength(1);
    expect(a!.targets[0]!.aria).toBe('aria Fatal');
    expect(a!.targets[0]!.leftPct).toBeCloseTo((a!.targets[0]!.left / 1000) * 100);
    expect(b!.filtered).toBe(true);
    expect(b!.title).toContain('remove the instance filter');
    expect(laneTitle('x', true, false)).toContain('remove the lane filter');
  });

  it('draws density bars scaled to the busiest bucket', () => {
    expect(densityPath([])).toBe('');
    expect(densityPath([0, 2, 1])).toBe('M1.1 0.00h0.8v16.00h-0.8zM2.1 7.00h0.8v9.00h-0.8z');
  });
});

describe('TimelineInteraction', () => {
  it('does not schedule change detection for keys pressed elsewhere', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(TimelinePanel);
    await settle(fixture);
    // The scheduler runs change detection through ApplicationRef._tick (internal).
    const tick = vi.spyOn(TestBed.inject(ApplicationRef) as unknown as { _tick(): void }, '_tick');
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', bubbles: true }));
    document.body.dispatchEvent(new KeyboardEvent('keyup', { key: 'a', bubbles: true }));
    await new Promise((r) => setTimeout(r, 30));
    expect(tick).not.toHaveBeenCalled();
    fixture.destroy();
  });
});
