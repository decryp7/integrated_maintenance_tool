import { vi } from 'vitest';
import { TestBed } from '@angular/core/testing';
import type { SerializedDockview } from 'dockview';
import { provideDockTesting, settle } from '../../testing/helpers';
import { DockService, PANEL } from '../dock/dock.service';
import { loadDockview } from '../dock/dockview-loader';
import { EMPTY_EXTRAS, LAYOUT_STORE, LayoutStore } from '../dock/layout-store';
import { provideTimelines } from './provide-timelines';
import { TimelineViews } from './timeline-views';

beforeAll(() => loadDockview());

describe('additional timelines in the dock', () => {
  let container: HTMLElement;
  let saved: SerializedDockview | null;

  function setup(fresh = true) {
    if (fresh) localStorage.clear();
    const store: LayoutStore = {
      load: () => (saved ? { layout: saved, extras: EMPTY_EXTRAS } : null),
      save: (l) => void (saved = l.layout),
      clear: () => void (saved = null),
    };
    TestBed.configureTestingModule({ providers: [provideDockTesting(), provideTimelines(), { provide: LAYOUT_STORE, useValue: store }] });
    container = document.createElement('div');
    document.body.appendChild(container);
    const dock = TestBed.inject(DockService);
    dock.attach(container);
    return { dock, views: TestBed.inject(TimelineViews) };
  }

  beforeEach(() => (saved = null));
  afterEach(() => {
    TestBed.inject(DockService).detach();
    container.remove();
  });

  const tabs = () => [...container.querySelectorAll('[role=tab]')].map((t) => t.getAttribute('aria-label'));

  it('opens a new timeline in its own group below the main timeline, with its title on the tab', async () => {
    const { dock, views } = setup();
    const groupsBefore = container.querySelectorAll('.dv-groupview').length;
    expect(views.open()).toBe('timeline#2');
    expect(dock.isOpen('timeline#2')).toBe(true);
    expect(dock.isOpen(PANEL.timeline)).toBe(true);
    expect(container.querySelectorAll('.dv-groupview').length).toBe(groupsBefore + 1);
    // The panel renders the timeline component, which names its tab.
    await vi.waitFor(() => expect(container.querySelectorAll('debrief-timeline-panel.instance').length).toBe(1));
    await vi.waitFor(() => expect(tabs()).toContain('Timeline 2'));
    views.rename('timeline#2', 'Errors DB01');
    await vi.waitFor(() => expect(tabs()).toContain('Errors DB01'));
    // Opening it again only focuses it.
    dock.showBeside('timeline#2', PANEL.timeline, 'below');
    expect([...dock.openIds()].filter((id) => id.startsWith('timeline'))).toEqual(['timeline', 'timeline#2']);
  });

  it('falls back to opening it as a tab when the main timeline is closed', () => {
    const { dock, views } = setup();
    dock.close(PANEL.timeline);
    views.open();
    expect(dock.isOpen('timeline#2')).toBe(true);
  });

  it('restores additional timelines with the saved layout and their settings', async () => {
    const first = setup();
    first.views.open({ title: 'Kept', scope: 'own', ql: 'level >= error' });
    const layout = first.dock.currentLayout();
    expect(Object.keys(layout!.layout.panels)).toContain('timeline#2');
    expect((layout!.layout.panels as Record<string, { contentComponent?: string }>)['timeline#2']?.contentComponent).toBe('timeline');
    first.dock.detach();
    container.remove();
    saved = layout!.layout;
    await settle();
    TestBed.resetTestingModule();

    const again = setup(false);
    expect(again.views.config('timeline#2')).toMatchObject({ title: 'Kept', scope: 'own', ql: 'level >= error' });
    expect(again.dock.isOpen('timeline#2')).toBe(true);
    await vi.waitFor(() => expect(container.querySelector('debrief-timeline-panel.instance')).not.toBeNull());
    await vi.waitFor(() => expect(tabs()).toContain('Kept'));
  });
});
