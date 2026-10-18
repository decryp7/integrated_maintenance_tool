import { Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideDockTesting, settle, tinyDataset } from '../../../testing/helpers';
import { MockBackend } from '../../core/mock/mock-backend';
import { WorkspaceStore } from '../../core/workspace.store';
import { DockService } from '../../dock/dock.service';
import { loadDockview } from '../../dock/dockview-loader';
import { LAYOUT_STORE, LayoutStore } from '../../dock/layout-store';
import { provideImtPlugin } from '../../extensions';

beforeAll(() => loadDockview());

@Component({ selector: 'imt-test-stub', template: 'stub' })
class Stub {}

/** The startup layout is the core views only; conditional views open by themselves only when they ask to. */
describe('default layout', () => {
  let container: HTMLElement;

  function setup() {
    const store: LayoutStore = { load: () => null, save: () => undefined, clear: () => undefined };
    TestBed.configureTestingModule({
      providers: [
        provideDockTesting(new MockBackend({ latency: false, dataset: tinyDataset })),
        { provide: LAYOUT_STORE, useValue: store },
        provideImtPlugin({
          views: [
            { id: 'offered', title: 'Offered', icon: 'file', category: 'Analysis', component: async () => Stub, when: (s) => s.total > 0 },
            { id: 'viewer', title: 'Viewer', icon: 'file', component: async () => Stub, when: (s) => s.total > 0, autoOpen: true },
            { id: 'tool', title: 'Tool', icon: 'file', category: 'Tools', component: async () => Stub },
          ],
        }),
      ],
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    return { dock: TestBed.inject(DockService), store: TestBed.inject(WorkspaceStore) };
  }

  afterEach(() => {
    TestBed.inject(DockService).detach();
    container.remove();
  });

  it('opens only the core views, plus views that ask to open with what was imported; reset gives the same', async () => {
    const { dock, store } = setup();
    dock.attach(container);
    expect([...dock.openIds()].sort()).toEqual(['events', 'inspector', 'timeline']);
    await settle();
    expect(store.summary()?.total).toBeGreaterThan(0);
    expect([...dock.openIds()].sort()).toEqual(['events', 'inspector', 'timeline', 'viewer']);
    dock.show('offered');
    dock.show('tool');
    dock.show('profile');
    expect(dock.isOpen('offered') && dock.isOpen('tool') && dock.isOpen('profile')).toBe(true);
    dock.reset();
    expect([...dock.openIds()].sort()).toEqual(['events', 'inspector', 'timeline', 'viewer']);
  });
});
