import { vi } from 'vitest';
import { TestBed } from '@angular/core/testing';
import { all, el, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../testing/helpers';
import { at } from '../../testing/must';
import { WorkspaceStore } from '../core/workspace.store';
import { TimeCursor } from '../analysis/time-cursor';
import { DOCK_PANEL_ID } from '../dock/angular-panel';
import { DockService } from '../dock/dock.service';
import { ContextMenuService } from '../ui/context-menu.service';
import { TimelinePanel } from './timeline-panel';
import { TimelineFakeDock } from './timeline-testing';
import { TimelineViews } from './timeline-views';
import { TimelineZoom } from './zoom';

const pointer = (type: string, clientX: number, extra: PointerEventInit = {}) => new PointerEvent(type, { bubbles: true, clientX, button: 0, pointerId: 1, ...extra });

describe('TimelinePanel as an additional timeline', () => {
  let dock: TimelineFakeDock;

  async function render(id: string | null = 'timeline#2') {
    localStorage.clear();
    dock = new TimelineFakeDock();
    if (id) dock.openIds.set(new Set(['timeline', id]));
    TestBed.configureTestingModule({
      providers: [provideTinyBackend(tinyBackend()), { provide: DockService, useValue: dock }, ...(id ? [{ provide: DOCK_PANEL_ID, useValue: id }] : [])],
    });
    const fixture = TestBed.createComponent(TimelinePanel);
    const store = TestBed.inject(WorkspaceStore);
    const views = TestBed.inject(TimelineViews);
    await settle(fixture);
    return { fixture, store, views };
  }

  type Fixture = Awaited<ReturnType<typeof render>>['fixture'];
  const select = (f: Fixture, label: string) => el<HTMLSelectElement>(f, `select[aria-label="${label}"]`);
  const brush = async (f: Fixture, x0: number, x1: number) => {
    const w = el(f, '.wrap');
    w.dispatchEvent(pointer('pointerdown', 250 + x0));
    w.dispatchEvent(pointer('pointermove', 250 + x1));
    w.dispatchEvent(pointer('pointerup', 250 + x1));
    await settle(f);
  };

  it('shows its title on the panel and its tab, with its own settings bar', async () => {
    const { fixture } = await render();
    expect(el(fixture, '[data-testid=timeline-title]').textContent).toBe('Timeline 2');
    expect(dock.titles.get('timeline#2')).toBe('Timeline 2');
    expect(el(fixture, '[role=group][aria-label="Timeline settings"]')).toBeTruthy();
    expect(select(fixture, 'Events shown').value).toBe('global');
    expect(fixture.nativeElement.querySelector('imt-live-window-controls')).toBeNull();
    expect(fixture.nativeElement.classList).toContain('instance');
    expect(text(fixture)).toContain('3 lanes');
    expect(text(fixture)).not.toContain('Unlinked');
  });

  it('uses its own query and lanes', async () => {
    const { fixture, views } = await render();
    typeInto(select(fixture, 'Events shown'), 'own', 'change');
    await settle(fixture);
    const ql = el<HTMLInputElement>(fixture, 'input[aria-label="Timeline query"]');
    typeInto(ql, 'level >= error', 'change');
    await settle(fixture);
    expect(views.config('timeline#2')).toMatchObject({ scope: 'own', ql: 'level >= error' });
    ql.value = 'level = fatal';
    ql.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(views.config('timeline#2').ql).toBe('level = fatal');

    typeInto(select(fixture, 'Lanes'), 'source', 'change');
    await settle(fixture);
    expect(all(fixture, '.lane-label .key').map((e) => e.textContent?.trim())).toEqual(['app.clef', 'app.log']);
    typeInto(select(fixture, 'Lanes'), 'field', 'change');
    await settle(fixture);
    typeInto(select(fixture, 'Lane field'), 'MachineName', 'change');
    await settle(fixture);
    expect(views.config('timeline#2')).toMatchObject({ lanes: 'field', laneField: 'MachineName' });
    expect(all(fixture, '.lane-label .key').map((e) => e.textContent?.trim())).toEqual(['WEB01', 'WEB02']);
    typeInto(select(fixture, 'Lanes'), 'bogus', 'change');
    expect(views.config('timeline#2').lanes).toBe('instance');
    typeInto(select(fixture, 'Events shown'), 'global', 'change');
    expect(views.config('timeline#2').scope).toBe('global');
  });

  it('shows saved settings in its controls', async () => {
    localStorage.setItem('imt.timelines', JSON.stringify({ version: 1, timelines: [{ id: 'timeline#2', scope: 'own', ql: 'level = fatal', lanes: 'field', laneField: 'MachineName', sync: false }] }));
    dock = new TimelineFakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(tinyBackend()), { provide: DockService, useValue: dock }, { provide: DOCK_PANEL_ID, useValue: 'timeline#2' }] });
    const fixture = TestBed.createComponent(TimelinePanel);
    await settle(fixture);
    expect(select(fixture, 'Events shown').value).toBe('own');
    expect(el<HTMLInputElement>(fixture, 'input[aria-label="Timeline query"]').value).toBe('level = fatal');
    expect(select(fixture, 'Lanes').value).toBe('field');
    expect(select(fixture, 'Lane field').value).toBe('MachineName');
    expect(el<HTMLInputElement>(fixture, 'imt-switch input').checked).toBe(false);
    expect(text(fixture)).toContain('Unlinked');
  });

  it('filters lanes and markers for itself only', async () => {
    const { fixture, store, views } = await render();
    const fatal = all<HTMLButtonElement>(fixture, '.legend .lg').find((b) => b.textContent?.includes('Fatal'))!;
    fatal.click();
    await settle(fixture);
    expect(views.config('timeline#2').markers).toEqual(['fatal']);
    expect(store.query().markers).toEqual([]);
    expect(fatal.getAttribute('aria-pressed')).toBe('true');
    fatal.click();
    expect(views.config('timeline#2').markers).toBeNull();
    await settle(fixture);
    at(all<HTMLButtonElement>(fixture, '.lane-label'), 1).click();
    await settle(fixture);
    expect(views.config('timeline#2').groups).toEqual(['WEB02 · 300']);
    expect(store.query().groups).toEqual([]);
    expect(el(fixture, '.lane-label.filtered').title).toContain('remove the lane filter');
  });

  it('renames from a double-click on the title; Escape cancels', async () => {
    const { fixture, views } = await render();
    el(fixture, '[data-testid=timeline-title]').dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    await settle(fixture);
    const input = el<HTMLInputElement>(fixture, 'input[aria-label="Timeline title"]');
    input.value = 'Errors WEB01';
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter' }));
    await settle(fixture);
    expect(views.config('timeline#2').title).toBe('Errors WEB01');
    expect(dock.titles.get('timeline#2')).toBe('Errors WEB01');
    expect(fixture.nativeElement.querySelector('input[aria-label="Timeline title"]')).toBeNull();

    el(fixture, '[data-testid=timeline-title]').dispatchEvent(new MouseEvent('dblclick', { bubbles: true }));
    await settle(fixture);
    const again = el<HTMLInputElement>(fixture, 'input[aria-label="Timeline title"]');
    again.value = 'nope';
    again.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }));
    again.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await settle(fixture);
    again.dispatchEvent(new FocusEvent('blur'));
    expect(views.config('timeline#2').title).toBe('Errors WEB01');
  });

  it('zooms the main timeline while synced and only itself once unlinked', async () => {
    const { fixture, store } = await render();
    await brush(fixture, 100, 300);
    expect(store.hasRange()).toBe(true);
    const shared = { from: store.query().from, to: store.query().to };

    // Unlink with the switch: the panel keeps the shared range, then moves on its own.
    const sw = el<HTMLInputElement>(fixture, 'imt-switch input');
    expect(sw.checked).toBe(true);
    sw.checked = false;
    sw.dispatchEvent(new Event('change'));
    await settle(fixture);
    expect(text(fixture)).toContain('Unlinked');
    expect(el(fixture, '.wrap').dataset['from']).toBe(String(shared.from));
    await brush(fixture, 100, 300);
    expect(store.query()).toMatchObject(shared);
    expect(Number(el(fixture, '.wrap').dataset['from'])).toBeGreaterThan(shared.from!);
    // Its cursor is its own.
    TestBed.inject(TimeCursor).hover(null);
    el(fixture, '.wrap').dispatchEvent(pointer('pointermove', 250 + 400));
    await new Promise((r) => requestAnimationFrame(r));
    expect(TestBed.inject(TimeCursor).hovered()).toBeNull();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('.cursor:not([hidden])')).not.toBeNull();
    el(fixture, '.wrap').dispatchEvent(pointer('pointerleave', 0));
    TestBed.inject(TimelineZoom).fit();
    await settle(fixture);
    expect(Number(el(fixture, '.wrap').dataset['from'])).toBeGreaterThan(shared.from!);
  });

  it('offers new timeline, rename, link and close in its menu', async () => {
    const { fixture, views } = await render();
    const menu = TestBed.inject(ContextMenuService);
    const open = () => {
      el<HTMLButtonElement>(fixture, 'button[aria-label="Timeline menu"]').click();
      return menu.menu()!.items;
    };
    expect(open().map((i) => i.label)).toEqual(['New timeline', 'Rename…', 'Unlink time from the main timeline', 'Close']);
    at(open(), 0).run();
    expect(dock.calls).toContain('beside:timeline#3:timeline:below');
    at(open(), 2).run();
    expect(views.config('timeline#2').sync).toBe(false);
    expect(at(open(), 2).label).toBe('Sync time with the main timeline');
    at(open(), 2).run();
    expect(views.config('timeline#2').sync).toBe(true);
    at(open(), 1).run();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('input[aria-label="Timeline title"]')).not.toBeNull();
    at(open(), 3).run();
    expect(dock.calls).toContain('close:timeline#2');
  });

  it('handles history keys only while focus is inside it', async () => {
    const { fixture, store } = await render();
    await brush(fixture, 100, 300);
    expect(store.hasRange()).toBe(true);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', altKey: true }));
    expect(store.hasRange()).toBe(true);
    const inside = el<HTMLButtonElement>(fixture, 'button[aria-label="Timeline menu"]');
    inside.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', altKey: true, bubbles: true }));
    expect(store.hasRange()).toBe(false);
  });

  it('a main timeline leaves history keys pressed inside an additional one to it', async () => {
    const { fixture, store } = await render(null);
    const zoom = TestBed.inject(TimelineZoom);
    zoom.apply({ from: store.timeline()!.from + 60_000, to: store.timeline()!.from + 120_000 });
    const other = document.createElement('imt-timeline-panel');
    other.className = 'instance';
    const btn = document.createElement('button');
    other.appendChild(btn);
    document.body.appendChild(other);
    btn.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', altKey: true, bubbles: true }));
    expect(store.hasRange()).toBe(true);
    document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', altKey: true, bubbles: true }));
    expect(store.hasRange()).toBe(false);
    other.remove();
    // The main timeline's menu only opens another timeline.
    el<HTMLButtonElement>(fixture, 'button[aria-label="Timeline menu"]').click();
    expect(TestBed.inject(ContextMenuService).menu()!.items.map((i) => i.label)).toEqual(['New timeline']);
    expect(fixture.nativeElement.querySelector('[role=group][aria-label="Timeline settings"]')).toBeNull();
  });

  it('shows its own failure inline', async () => {
    localStorage.clear();
    const backend = tinyBackend();
    const real = backend.queryTimeline.bind(backend);
    vi.spyOn(backend, 'queryTimeline').mockImplementation((req) => (req.query.ql === 'bad' ? Promise.reject(new Error('query: unexpected end')) : real(req)));
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), { provide: DockService, useValue: new TimelineFakeDock() }, { provide: DOCK_PANEL_ID, useValue: 'timeline#2' }] });
    TestBed.inject(TimelineViews).update('timeline#2', { scope: 'own', ql: 'bad' });
    const fixture = TestBed.createComponent(TimelinePanel);
    await settle(fixture);
    expect(el(fixture, '.err-card').textContent).toContain('query: unexpected end');
    expect(TestBed.inject(WorkspaceStore).timelineError()).toBeNull();
  });
});
