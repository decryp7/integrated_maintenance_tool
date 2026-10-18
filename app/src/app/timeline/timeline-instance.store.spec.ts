import { vi } from 'vitest';
import { EnvironmentInjector, Injector } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle, tinyBackend } from '../../testing/helpers';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { TimeCursor } from '../analysis/time-cursor';
import { DOCK_PANEL_ID } from '../dock/angular-panel';
import { DockService } from '../dock/dock.service';
import { TimelineInstanceStore, TimelineInstanceStoreType } from './timeline-instance.store';
import { MainTimelineSource } from './timeline-source';
import { TimelineViews } from './timeline-views';
import { TimelineFakeDock } from './timeline-testing';
import { TimelineZoom } from './zoom';

describe('TimelineInstanceStore', () => {
  let backend: MockBackend;
  let injectors: Injector[];

  beforeEach(() => {
    localStorage.clear();
    backend = tinyBackend();
    injectors = [];
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), { provide: DockService, useValue: new TimelineFakeDock() }] });
  });

  afterEach(() => injectors.forEach((i) => (i as unknown as { destroy(): void }).destroy?.()));

  /** One panel's store, as the panel component provides it. */
  function panel(id: string): TimelineInstanceStoreType {
    const injector = Injector.create({ providers: [TimelineInstanceStore, { provide: DOCK_PANEL_ID, useValue: id }], parent: TestBed.inject(EnvironmentInjector) });
    injectors.push(injector);
    return injector.get(TimelineInstanceStore);
  }

  /** Lanes with matching events (the engine also reports the others, with their totals). */
  const keys = (s: TimelineInstanceStoreType) => s.timeline()?.lanes.filter((l) => l.matched > 0).map((l) => l.key);

  async function loaded() {
    const ws = TestBed.inject(WorkspaceStore);
    const views = TestBed.inject(TimelineViews);
    await settle();
    return { ws, views };
  }

  it('asks the engine for its own query and keeps the workspace timeline as it is', async () => {
    const { ws, views } = await loaded();
    const spy = vi.spyOn(backend, 'queryTimeline');
    const s = panel('timeline#2');
    views.update('timeline#2', { scope: 'own', ql: 'level >= error' });
    await settle();
    expect(spy.mock.calls.at(-1)?.[0].query.ql).toBe('level >= error');
    expect(keys(s)).toEqual(['WEB01 · 100']);
    expect(ws.timeline()?.lanes).toHaveLength(3);
    expect(s.main).toBe(false);
    expect(s.title()).toBe('Timeline 2');
    expect(s.loading()).toBe(false);
  });

  it('keeps the state of each panel apart', async () => {
    const { views } = await loaded();
    const a = panel('timeline#2');
    const b = panel('timeline#3');
    views.update('timeline#2', { lanes: 'source' });
    views.update('timeline#3', { lanes: 'level' });
    await settle();
    expect(keys(a)).toEqual(['app.clef', 'app.log']);
    expect(keys(b)?.sort()).toEqual(['Debug', 'Error', 'Fatal', 'Information', 'Warning']);
    expect(a.groupBy()).toEqual(['@source']);
    a.setBuckets(100);
    expect(a.buckets()).toBe(100);
    expect(b.buckets()).toBe(400);
  });

  it('follows the workspace filter while it inherits it', async () => {
    const { ws } = await loaded();
    const s = panel('timeline#2');
    await settle();
    expect(keys(s)).toHaveLength(3);
    ws.toggleLevel('Error');
    await settle();
    expect(keys(s)).toEqual(['WEB01 · 100']);
  });

  it('does not ask again for workspace changes that do not affect it', async () => {
    const { ws, views } = await loaded();
    panel('timeline#2');
    views.update('timeline#2', { scope: 'own', ql: '' });
    await settle();
    const spy = vi.spyOn(backend, 'queryTimeline');
    ws.toggleLevel('Error');
    await settle();
    // Only the workspace's own timeline is fetched again.
    expect(spy).toHaveBeenCalledTimes(1);
  });

  it('filters markers and lanes in its own settings', async () => {
    const { ws, views } = await loaded();
    const s = panel('timeline#2');
    ws.patchQuery({ markers: ['fatal'] });
    expect(s.markers()).toEqual(['fatal']);
    s.showOnlyMarker('error');
    expect(views.config('timeline#2').markers).toEqual(['error']);
    expect(s.markers()).toEqual(['error']);
    s.showOnlyMarker(null);
    expect(s.markers()).toEqual(['fatal']);
    s.toggleGroup('WEB02 · 300');
    expect(s.groups()).toEqual(['WEB02 · 300']);
    s.toggleGroup('WEB02 · 300');
    expect(s.groups()).toEqual([]);
    s.toggleGroup('x');
    s.setLanes('field', 'MachineName');
    expect(views.config('timeline#2')).toMatchObject({ lanes: 'field', laneField: 'MachineName', groups: [] });
    s.setLanes('level');
    expect(views.config('timeline#2')).toMatchObject({ lanes: 'level', laneField: 'MachineName' });
    s.setScope('own');
    s.setQl('level = fatal');
    s.rename('Fatal only');
    expect(views.config('timeline#2')).toMatchObject({ scope: 'own', ql: 'level = fatal', title: 'Fatal only' });
    expect(ws.query().markers).toEqual(['fatal']);
  });

  it('zooms with the main timeline and shares the cursor while synced', async () => {
    const { ws } = await loaded();
    const s = panel('timeline#2');
    await settle();
    const zoom = TestBed.inject(TimelineZoom);
    expect(s.zoom()).toBe(zoom);
    expect(s.cursor()).toBe(TestBed.inject(TimeCursor));
    const t = ws.timeline()!;
    const mid = { from: t.from + 60_000, to: t.from + 180_000 };
    s.zoom().apply(mid);
    await settle();
    expect(ws.query()).toMatchObject(mid);
    expect(s.request()).toMatchObject(mid);
    expect(s.hasRange()).toBe(true);
    expect(s.timeline()?.from).toBe(ws.timeline()?.from);
    expect(s.timeline()?.to).toBe(ws.timeline()?.to);
  });

  it('keeps its own range, history and cursor once unlinked, starting from the main view', async () => {
    const { ws } = await loaded();
    const s = panel('timeline#2');
    await settle();
    const zoom = TestBed.inject(TimelineZoom);
    const t = ws.timeline()!;
    const shared = { from: t.from + 60_000, to: t.from + 300_000 };
    zoom.apply(shared);
    s.setSync(false);
    s.setSync(false);
    expect(s.synced()).toBe(false);
    expect(s.range()).toEqual(shared);
    expect(s.zoom()).not.toBe(zoom);
    expect(s.cursor()).not.toBe(TestBed.inject(TimeCursor));

    // Zooming the unlinked panel leaves the workspace alone, and the other way round.
    const own = { from: t.from + 100_000, to: t.from + 200_000 };
    s.zoom().apply(own);
    expect(ws.query()).toMatchObject(shared);
    expect(s.request()).toMatchObject(own);
    zoom.fit();
    expect(s.request()).toMatchObject(own);
    expect(s.zoom().canBack()).toBe(true);
    s.zoom().back();
    expect(s.range()).toBeNull();
    expect(s.hasRange()).toBe(false);
    s.zoom().forward();
    expect(s.range()).toEqual(own);
    s.cursor().hover(t.from + 1);
    expect(TestBed.inject(TimeCursor).hovered()).toBeNull();
    await settle();
    expect(s.timeline()?.from).toBe(own.from);

    // Linking again follows the main timeline.
    s.setSync(true);
    expect(s.range()).toBeNull();
    expect(s.zoom()).toBe(zoom);
    expect(s.request()).toMatchObject({ from: null, to: null });
  });

  it('unlinks to the full range when the main timeline is not zoomed', async () => {
    await loaded();
    const s = panel('timeline#2');
    s.setSync(false);
    expect(s.range()).toBeNull();
    expect(s.zoom().view()).toEqual(TestBed.inject(TimelineZoom).extent());
    expect(s.zoom().canPan()).toBe(false);
  });

  it('shows a failed request and fetches again on retry', async () => {
    await loaded();
    vi.spyOn(backend, 'queryTimeline').mockRejectedValueOnce(new Error('query: unexpected token'));
    const s = panel('timeline#2');
    await settle();
    expect(s.error()).toBe('query: unexpected token');
    s.retry();
    await settle();
    expect(s.error()).toBeNull();
    expect(s.timeline()).not.toBeNull();
  });

  it('reports non-Error failures as text', async () => {
    await loaded();
    const spy = vi.spyOn(backend, 'queryTimeline').mockRejectedValueOnce('plain');
    const s = panel('timeline#2');
    await settle();
    expect(s.error()).toBe('plain');
    spy.mockRejectedValueOnce({ code: 1 });
    s.retry();
    await settle();
    expect(s.error()).toBe('{"code":1}');
  });

  it('waits for a workspace before asking', () => {
    const spy = vi.spyOn(backend, 'queryTimeline');
    panel('timeline#2');
    TestBed.tick();
    expect(spy).not.toHaveBeenCalled();
  });
});

describe('MainTimelineSource', () => {
  it('drives the workspace', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const src = TestBed.inject(MainTimelineSource);
    const ws = TestBed.inject(WorkspaceStore);
    await settle();
    expect(src.main).toBe(true);
    expect(src.title()).toBe('Timeline');
    expect(src.timeline()).toBe(ws.timeline());
    src.showOnlyMarker('fatal');
    expect(ws.query().markers).toEqual(['fatal']);
    expect(src.markers()).toEqual(['fatal']);
    src.showOnlyMarker(null);
    expect(ws.query().markers).toEqual([]);
    src.toggleGroup('WEB01 · 100');
    expect(src.groups()).toEqual(['WEB01 · 100']);
    src.setBuckets(100);
    expect(ws.buckets()).toBe(100);
    src.retry();
    expect(src.zoom()).toBe(TestBed.inject(TimelineZoom));
    expect(src.cursor()).toBe(TestBed.inject(TimeCursor));
  });
});
