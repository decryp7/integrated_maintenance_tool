import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it } from 'vitest';
import { all, settle } from '../../testing/helpers';
import { DumpMarker } from '../core/api.types';
import { DumpMarkerTrack, layoutMarkers } from './dump-marker-track';
import { DumpsBackend } from './dumps-backend';
import { DumpsStore } from './dumps.store';
import { setupDumps } from './testing/dumps-test-support';

const marker = (id: number, timeMs: number, kind: DumpMarker['kind'] = 'dump'): DumpMarker => ({ id, name: `m${id}`, kind, timeMs, headline: { type: 'exception', name: 'h', location: null }, severity: 'critical' });

describe('layoutMarkers', () => {
  it('places markers in the range and stacks nearby labels', () => {
    const vm = layoutMarkers([marker(2, 500), marker(1, 501), marker(3, 2000)], { from: 0, to: 1000 }, 1000);
    expect(vm.map((m) => m.marker.id)).toEqual([2, 1]);
    expect(vm[0]!.left).toBe(50);
    expect(vm.map((m) => m.row)).toEqual([0, 1]);
    expect(layoutMarkers([marker(1, 5)], { from: 5, to: 5 }, 100)[0]!.left).toBe(0);
  });
});

@Component({
  imports: [DumpMarkerTrack],
  template: `<imt-dump-marker-track [view]="view()" [width]="800" />`,
})
class Host {
  readonly view = signal<{ from: number; to: number } | null>(null);
}

describe('DumpMarkerTrack', () => {
  it('shows the open documents at their time and opens them', async () => {
    const t = setupDumps();
    const store = TestBed.inject(DumpsStore);
    await store.pickAndOpen('dump');
    await store.pickAndOpen('umdh');
    const fixture = TestBed.createComponent(Host);
    const crash = store.dump()!.timeMs!;
    fixture.componentInstance.view.set({ from: crash - 3_600_000, to: crash + 3_600_000 });
    fixture.detectChanges();
    await settle(fixture, 6);
    const chips = all<HTMLButtonElement>(fixture, '[data-testid="dump-marker"]');
    expect(chips).toHaveLength(4);
    expect(chips.some((c) => c.getAttribute('aria-label') === 'Open dump OrderService.exe.4120.dmp')).toBe(true);
    expect(chips[0]!.title).toContain('Click to open.');
    chips.find((c) => c.textContent?.includes('OrderService.exe.4120.dmp'))!.click();
    chips.find((c) => c.textContent?.includes('snap-1035'))!.click();
    expect(t.context.calls.filter((c) => c.fn === 'showView').map((c) => c.args[0])).toEqual(['dump', 'umdh']);
    expect(store.umdhTarget()).toBe(store.umdh()[1]!.id);
  });

  it('renders nothing without a range, and keeps working when markers fail', async () => {
    const t = setupDumps();
    t.backend.markers = () => Promise.reject(new Error('down'));
    const fixture = TestBed.createComponent(Host);
    fixture.detectChanges();
    await settle(fixture, 4);
    expect(all(fixture, '[data-testid="dump-marker"]')).toHaveLength(0);
    fixture.componentInstance.view.set({ from: 0, to: 1 });
    fixture.detectChanges();
    await settle(fixture, 4);
    expect(all(fixture, '[data-testid="dump-marker"]')).toHaveLength(0);
  });

  it('renders nothing without the dumps feature', async () => {
    setupDumps();
    TestBed.overrideProvider(DumpsBackend, { useValue: null });
    const fixture = TestBed.createComponent(Host);
    fixture.componentInstance.view.set({ from: 0, to: 1 });
    fixture.detectChanges();
    await settle(fixture, 4);
    expect(all(fixture, '[data-testid="dump-marker"]')).toHaveLength(0);
  });
});
