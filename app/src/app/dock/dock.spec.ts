import { ApplicationRef, ChangeDetectionStrategy, Component, effect, EnvironmentInjector, ErrorHandler, inject, OnDestroy, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import type { DockviewApi, SerializedDockview } from 'dockview';
import { MemoryLogger, provideDockTesting, settle, testLogger, tinyDataset } from '../../testing/helpers';
import { FailureReporter } from '../core/failure-reporter';
import { Logger } from '../core/logger';
import { MockBackend } from '../core/mock/mock-backend';
import { MockDataset } from '../core/mock/mock-data';
import { WorkspaceStore } from '../core/workspace.store';
import { REGISTRY_PLUGIN } from '../plugins/registry/registry.plugin';
import { ImtPlugin, provideImtPlugin } from '../extensions';
import { AngularContentRenderer, MissingViewRenderer } from './angular-panel';
import { PANEL_VISIBLE } from '../core/panel-visibility';
import { DockHost } from './dock-host';
import { DockService, keepGroupsUnscrolled, PANEL, QUERY_HEIGHT, QUERY_VIEW } from './dock.service';
import { GroupActions } from './group-actions';
import { EMPTY_EXTRAS, LAYOUT_KEY, LAYOUT_STORE, ENVELOPE_VERSION, LayoutStore, LocalStorageLayoutStore } from './layout-store';
import { PanelTab, tabPanelId } from './panel-tab';
import { loadDockview } from './dockview-loader';
import { at, must } from '../../testing/must';

// dockview is loaded on demand (see dockview-loader); load it once so attach() is synchronous here.
beforeAll(() => loadDockview());

@Component({ selector: 'imt-test-query', template: 'query bar' })
class QueryStub {}

/** Stand-in for the analysis plugin's query bar (a conditional bottom view), plus a right-hand tool window opened on demand. */
const QUERY_PLUGIN: ImtPlugin = {
  views: [
    { id: QUERY_VIEW, title: 'Query', icon: 'search', defaultPosition: 'bottom', component: async () => QueryStub, when: (s) => s.total > 0, autoOpen: true },
    { id: 'test-tool', title: 'Tool', icon: 'panel', defaultPosition: 'right', component: async () => QueryStub },
  ],
};

/** Ids of the panels in each group, in grid order (depth first). */
function groupsInOrder(layout: SerializedDockview): string[][] {
  const out: string[][] = [];
  const walk = (node: { type: string; data: unknown }) => {
    if (node.type === 'leaf') out.push((node.data as { views: string[] }).views);
    else (node.data as { type: string; data: unknown }[]).forEach(walk);
  };
  walk(layout.grid.root as never);
  return out;
}

function registryDataset(): MockDataset {
  const d = tinyDataset();
  d.sources.push({ id: 5, path: '/r.reg', name: 'r.reg', importer: 'registry', skipped: 0 });
  d.records.push({ id: 0, ts: null, level: 'Information', message: 'HKLM\\K', template: null, exception: null, fields: { Hive: 'HKLM', Key: 'K', ValueType: 'KEY' }, source: 5, line: 1, markers: [] });
  return d;
}

const LAYOUT: SerializedDockview = { grid: { root: { type: 'leaf', data: { views: [], id: '1' } as never }, height: 1, width: 1, orientation: 'HORIZONTAL' as never }, panels: {} };

describe('LocalStorageLayoutStore', () => {
  beforeEach(() => localStorage.clear());

  it('saves and loads a versioned layout', () => {
    const store = new LocalStorageLayoutStore();
    expect(store.load()).toBeNull();
    store.save({ layout: LAYOUT, extras: EMPTY_EXTRAS });
    expect(JSON.parse(localStorage.getItem(LAYOUT_KEY)!).version).toBe(ENVELOPE_VERSION);
    expect(store.load()).toEqual({ layout: LAYOUT, extras: EMPTY_EXTRAS });
    store.clear();
    expect(store.load()).toBeNull();
  });

  it('ignores incompatible, malformed or empty entries', () => {
    const store = new LocalStorageLayoutStore();
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ version: ENVELOPE_VERSION + 1, layout: LAYOUT }));
    expect(store.load()).toBeNull();
    localStorage.setItem(LAYOUT_KEY, 'null');
    expect(store.load()).toBeNull();
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ version: ENVELOPE_VERSION }));
    expect(store.load()).toBeNull();
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ version: ENVELOPE_VERSION, layout: 'x' }));
    expect(store.load()).toBeNull();
    localStorage.setItem(LAYOUT_KEY, '{not json');
    expect(store.load()).toBeNull();
  });

  it('survives unavailable storage', () => {
    const store = new LocalStorageLayoutStore();
    const spies = [
      vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked'); }),
      vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('quota'); }),
      vi.spyOn(Storage.prototype, 'removeItem').mockImplementation(() => { throw new Error('blocked'); }),
    ];
    expect(store.load()).toBeNull();
    expect(() => store.save({ layout: LAYOUT, extras: EMPTY_EXTRAS })).not.toThrow();
    expect(() => store.clear()).not.toThrow();
    spies.forEach((s) => s.mockRestore());
  });

  it('is the default LAYOUT_STORE', () => {
    expect(TestBed.inject(LAYOUT_STORE)).toBeInstanceOf(LocalStorageLayoutStore);
  });
});

describe('AngularContentRenderer', () => {
  @Component({ selector: 'imt-probe', template: 'probe-content' })
  class Probe implements OnDestroy {
    static destroyed = 0;
    ngOnDestroy() {
      Probe.destroyed++;
    }
  }

  function renderer(load: () => Promise<never>) {
    return new AngularContentRenderer(load, TestBed.inject(ApplicationRef), TestBed.inject(EnvironmentInjector), 'probe');
  }

  beforeEach(() => (Probe.destroyed = 0));

  it('creates the component lazily, attaches it to the application while visible and destroys it on dispose', async () => {
    const r = renderer(async () => Probe as never);
    const app = TestBed.inject(ApplicationRef);
    const before = app.viewCount;
    r.init({} as never);
    await vi.waitFor(() => expect(r.element.textContent).toBe('probe-content'));
    // Attached: Angular's dirty tracking refreshes it (and skips it while it is clean).
    expect(app.viewCount).toBe(before + 1);
    r.dispose();
    expect(app.viewCount).toBe(before);
    expect(Probe.destroyed).toBe(1);
    expect(r.element.textContent).toBe('');
  });

  it('waits until the panel is visible before creating the component', async () => {
    const r = renderer(async () => Probe as never);
    let notify!: (e: { isVisible: boolean }) => void;
    const dispose = vi.fn();
    r.init({ api: { isVisible: false, onDidVisibilityChange: (cb: typeof notify) => ((notify = cb), { dispose }) } } as never);
    await new Promise((x) => setTimeout(x, 0));
    expect(r.element.textContent).toBe('');
    notify({ isVisible: false });
    notify({ isVisible: true });
    notify({ isVisible: true });
    await vi.waitFor(() => expect(r.element.textContent).toBe('probe-content'));
    // It keeps listening (a hidden tab is detached from change detection) until the panel goes away.
    expect(dispose).not.toHaveBeenCalled();
    r.dispose();
    expect(dispose).toHaveBeenCalled();
    expect(Probe.destroyed).toBe(1);
  });

  it('does not create anything when disposed before the component loaded', async () => {
    let release!: (t: never) => void;
    const r = renderer(() => new Promise((res) => (release = res)));
    r.init({} as never);
    r.dispose();
    release(Probe as never);
    await new Promise((x) => setTimeout(x, 0));
    expect(r.element.textContent).toBe('');
    expect(Probe.destroyed).toBe(0);
  });

  it('shows an alert when the component cannot be loaded, and stays quiet if already disposed', async () => {
    const r = renderer(async () => Promise.reject(new Error('chunk')));
    r.init({} as never);
    await vi.waitFor(() => expect(r.element.querySelector('[role=alert]')?.textContent).toContain('This panel failed: chunk'));
    r.dispose();
    const gone = renderer(async () => Promise.reject(new Error('chunk')));
    gone.init({} as never);
    gone.dispose();
    await new Promise((x) => setTimeout(x, 0));
    expect(gone.element.textContent).toBe('');
  });

  it('MissingViewRenderer explains what happened', () => {
    const m = new MissingViewRenderer('gone');
    m.init();
    expect(m.element.textContent).toContain('"gone" is not available');
  });
});

@Component({ selector: 'imt-boom-ctor', template: 'never' })
class BoomCtor {
  constructor() {
    throw new Error('constructor boom');
  }
}

@Component({ selector: 'imt-boom-render', template: '{{ boom() }}' })
class BoomRender {
  boom(): string {
    throw new Error('render boom');
  }
}

@Component({ selector: 'imt-boom-later', changeDetection: ChangeDetectionStrategy.OnPush, template: '{{ show() ? boom() : "fine" }}' })
class BoomLater {
  static current: BoomLater | null = null;
  readonly show = signal(false);
  constructor() {
    BoomLater.current = this;
  }
  boom(): string {
    throw new Error('later boom');
  }
}

@Component({ selector: 'imt-boom-effect', template: 'effect panel' })
class BoomEffect {
  constructor() {
    effect(() => {
      throw new Error('effect boom');
    });
  }
}

@Component({ selector: 'imt-counter', changeDetection: ChangeDetectionStrategy.OnPush, template: '<span class="n">{{ n() }}</span>' })
class Counter {
  static current: Counter | null = null;
  readonly n = signal(0);
  constructor() {
    Counter.current = this;
  }
}

/** A panel that counts how often Angular checks its host binding and runs its template. */
@Component({
  selector: 'imt-check-probe',
  host: { '[attr.data-host-checks]': 'hostCheck()' },
  template: '<span class="v">{{ render() }}</span>',
})
class CheckProbe {
  static current: CheckProbe | null = null;
  static hostChecks = 0;
  static renders = 0;
  readonly value = signal(0);
  readonly visible = inject(PANEL_VISIBLE, { optional: true }) ?? signal(true);
  readonly effects = signal(0);
  constructor() {
    CheckProbe.current = this;
    effect(() => {
      this.value();
      this.effects.update((n) => n + 1);
    });
  }
  hostCheck(): number {
    return ++CheckProbe.hostChecks;
  }
  render(): number {
    CheckProbe.renders++;
    return this.value();
  }
}

describe('Panel change detection', () => {
  beforeEach(() => {
    CheckProbe.hostChecks = 0;
    CheckProbe.renders = 0;
  });

  function renderer(type: unknown) {
    return new AngularContentRenderer(async () => type as never, TestBed.inject(ApplicationRef), TestBed.inject(EnvironmentInjector), 'probe');
  }

  /** A visibility api like dockview's, whose changes the test drives. */
  function api(isVisible: boolean) {
    let notify: (e: { isVisible: boolean }) => void = () => undefined;
    return {
      params: { api: { id: 'p', isVisible, onDidVisibilityChange: (cb: typeof notify) => ((notify = cb), { dispose: () => undefined }) } } as never,
      show: (v: boolean) => notify({ isVisible: v }),
    };
  }

  it('does not check a clean panel when another panel changes', async () => {
    const busy = renderer(Counter);
    busy.init({} as never);
    const quiet = renderer(CheckProbe);
    quiet.init({} as never);
    await vi.waitFor(() => expect(quiet.element.textContent).toBe('0'));
    await TestBed.inject(ApplicationRef).whenStable();
    const hostChecks = CheckProbe.hostChecks;
    const renders = CheckProbe.renders;
    for (let i = 1; i <= 5; i++) {
      Counter.current!.n.set(i);
      await vi.waitFor(() => expect(busy.element.textContent).toBe(String(i)));
    }
    // Before: every render of the application re-checked every panel (5 more host checks here).
    expect(CheckProbe.hostChecks).toBe(hostChecks);
    expect(CheckProbe.renders).toBe(renders);
    // Its own signal still updates it.
    CheckProbe.current!.value.set(7);
    await vi.waitFor(() => expect(quiet.element.textContent).toBe('7'));
    busy.dispose();
    quiet.dispose();
  });

  it('does not check a hidden panel, and brings it up to date when it is shown again', async () => {
    const app = TestBed.inject(ApplicationRef);
    const tab = api(true);
    const r = renderer(CheckProbe);
    r.init(tab.params);
    await vi.waitFor(() => expect(r.element.textContent).toBe('0'));
    const probe = CheckProbe.current!;
    await app.whenStable();
    expect(probe.visible()).toBe(true);
    tab.show(false);
    expect(probe.visible()).toBe(false);
    await app.whenStable();
    const renders = CheckProbe.renders;
    const effects = probe.effects();
    probe.value.set(3);
    await app.whenStable();
    await new Promise((x) => setTimeout(x, 20));
    expect(r.element.textContent).toBe('0');
    expect(CheckProbe.renders).toBe(renders);
    expect(probe.effects()).toBe(effects);
    tab.show(true);
    await vi.waitFor(() => expect(r.element.textContent).toBe('3'));
    expect(probe.effects()).toBe(effects + 1);
    r.dispose();
  });

  it('mounts a panel that starts hidden when it is first shown; setVisible hides and shows it', async () => {
    const app = TestBed.inject(ApplicationRef);
    const views = app.viewCount;
    const tab = api(false);
    const r = renderer(CheckProbe);
    r.init(tab.params);
    await new Promise((x) => setTimeout(x, 0));
    expect(app.viewCount).toBe(views);
    tab.show(true);
    await vi.waitFor(() => expect(r.element.textContent).toBe('0'));
    expect(app.viewCount).toBe(views + 1);
    const probe = CheckProbe.current!;
    r.setVisible(false);
    expect(probe.visible()).toBe(false);
    probe.value.set(4);
    await app.whenStable();
    expect(r.element.textContent).toBe('0');
    r.setVisible(true);
    await vi.waitFor(() => expect(r.element.textContent).toBe('4'));
    r.dispose();
    expect(app.viewCount).toBe(views);
    r.setVisible(true);
    expect(r.element.textContent).toBe('');
  });
});

describe('Panel error boundary', () => {
  let logger: MemoryLogger;
  beforeEach(() => {
    logger = new MemoryLogger();
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useValue: logger }] });
  });

  function renderer(type: unknown, label = 'probe') {
    return new AngularContentRenderer(async () => type as never, TestBed.inject(ApplicationRef), TestBed.inject(EnvironmentInjector), label);
  }

  const card = (r: AngularContentRenderer) => r.element.querySelector<HTMLElement>('.imt-panel-failed');
  const failed = async (r: AngularContentRenderer, message: string) => {
    r.init({} as never);
    await vi.waitFor(() => expect(card(r)?.textContent).toContain(`This panel failed: ${message}`));
  };

  it.each([
    ['a constructor', BoomCtor, 'constructor boom'],
    ['the first render', BoomRender, 'render boom'],
    ['an effect', BoomEffect, 'effect boom'],
  ])('contains an error in %s: shows the card, logs it, and leaves other panels alone', async (_what, type, message) => {
    const healthy = renderer(Counter, 'healthy');
    healthy.init({} as never);
    await vi.waitFor(() => expect(healthy.element.textContent).toBe('0'));

    const broken = renderer(type, 'broken');
    broken.init({} as never);
    await vi.waitFor(() => expect(card(broken)?.textContent).toContain(`This panel failed: ${message}`));
    expect(card(broken)?.getAttribute('role')).toBe('alert');
    expect(card(broken)?.textContent).toContain('Reload panel');
    expect(card(broken)?.textContent).toContain('Copy details');
    expect(logger.entries.filter((e) => e.level === 'error' && e.category === 'ui.panel')).toEqual([
      expect.objectContaining({ message: 'Panel failed', props: expect.objectContaining({ Panel: 'broken', Error: message }) }),
    ]);

    // The healthy panel keeps updating after the broken one failed.
    Counter.current!.n.set(5);
    await vi.waitFor(() => expect(healthy.element.textContent).toBe('5'));
    healthy.dispose();
    broken.dispose();
  });

  it('contains an error on a later render, after the panel worked', async () => {
    const r = renderer(BoomLater);
    r.init({} as never);
    await vi.waitFor(() => expect(r.element.textContent).toBe('fine'));
    BoomLater.current!.show.set(true);
    await vi.waitFor(() => expect(card(r)?.textContent).toContain('later boom'));
    expect(r.element.querySelector('imt-boom-later')).toBeNull();
    r.dispose();
  });

  it('keeps refreshing a healthy panel when its signals change', async () => {
    const r = renderer(Counter);
    r.init({} as never);
    await vi.waitFor(() => expect(r.element.textContent).toBe('0'));
    Counter.current!.n.set(1);
    await vi.waitFor(() => expect(r.element.textContent).toBe('1'));
    Counter.current!.n.set(2);
    await vi.waitFor(() => expect(r.element.textContent).toBe('2'));
    r.dispose();
  });

  it('reports only the first of several errors', async () => {
    const r = renderer(BoomEffect);
    await failed(r, 'effect boom');
    expect(logger.entries.filter((e) => e.level === 'error')).toHaveLength(1);
    r.dispose();
  });

  it('Reload panel tries again with a new component', async () => {
    let attempts = 0;
    const r = new AngularContentRenderer(
      async () => (++attempts === 1 ? BoomCtor : Counter) as never,
      TestBed.inject(ApplicationRef),
      TestBed.inject(EnvironmentInjector),
      'probe',
    );
    await failed(r, 'constructor boom');
    at(card(r)!.querySelectorAll('button'), 0).click();
    await vi.waitFor(() => expect(r.element.textContent).toBe('0'));
    expect(card(r)).toBeNull();
    expect(attempts).toBe(2);
    r.dispose();
  });

  it('Copy details puts the panel, message and stack on the clipboard', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const r = renderer(BoomCtor, 'events');
    await failed(r, 'constructor boom');
    const copy = at(card(r)!.querySelectorAll('button'), 1);
    copy.click();
    await vi.waitFor(() => expect(copy.textContent).toBe('Copied'));
    expect(at(writeText.mock.calls, 0)[0]).toContain('Panel: events\nError: constructor boom');
    writeText.mockRejectedValueOnce(new Error('denied'));
    copy.click();
    await vi.waitFor(() => expect(copy.textContent).toBe('Copy failed'));
    vi.unstubAllGlobals();
    r.dispose();
  });

  it('reports the failure to the FailureReporter after the card is shown, and survives a broken reporter', async () => {
    const seen: [string, unknown, boolean][] = [];
    let r: AngularContentRenderer | null = null;
    const reporter = {
      panelFailed: vi.fn((panel: string, error: unknown) => {
        seen.push([panel, error, !!r && card(r) !== null]);
        throw new Error('reporter broke');
      }),
      backendFailed: vi.fn(),
    };
    TestBed.resetTestingModule();
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useValue: logger }, { provide: FailureReporter, useValue: reporter }] });
    r = renderer(BoomCtor, 'broken');
    await failed(r, 'constructor boom');
    expect(seen).toHaveLength(1);
    expect(at(seen, 0)[0]).toBe('broken');
    expect((at(seen, 0)[1] as Error).message).toBe('constructor boom');
    expect(at(seen, 0)[2]).toBe(true);
    expect(reporter.backendFailed).not.toHaveBeenCalled();
    r.dispose();
  });

  it('describes errors that are not Error objects', async () => {
    @Component({ selector: 'imt-boom-string', template: '{{ boom() }}' })
    class BoomString {
      boom(): string {
        throw 'plain string';
      }
    }
    const r = renderer(BoomString);
    await failed(r, 'plain string');
    r.dispose();
  });

  it('routes errors the framework reports through ErrorHandler to the panel', async () => {
    @Component({ selector: 'imt-reports', template: 'reports' })
    class Reports {
      constructor() {
        const handler = inject(ErrorHandler);
        queueMicrotask(() => handler.handleError(undefined));
      }
    }
    const r = renderer(Reports);
    await failed(r, 'unknown error');
    r.dispose();
  });

  it('does not show a card for a panel disposed before the failure was handled', async () => {
    const r = renderer(Counter);
    r.init({} as never);
    await vi.waitFor(() => expect(r.element.textContent).toBe('0'));
    (r as unknown as { fail(e: unknown): void }).fail(new Error('late'));
    (r as unknown as { fail(e: unknown): void }).fail(new Error('second is ignored'));
    r.dispose();
    await new Promise((x) => setTimeout(x, 10));
    expect(r.element.textContent).toBe('');
    expect(logger.entries).toEqual([]);
    (r as unknown as { fail(e: unknown): void }).fail(new Error('after dispose'));
    expect(logger.entries).toEqual([]);
  });
});

describe('PanelTab', () => {
  it('renders the title, follows renames and closes the panel from the x', () => {
    let rename!: (e: { title: string }) => void;
    const close = vi.fn();
    const dispose = vi.fn();
    const tab = new PanelTab();
    tab.init({ title: 'Timeline', api: { id: 'timeline', close, onDidTitleChange: (cb: typeof rename) => ((rename = cb), { dispose }) } } as never);
    expect(tab.element.textContent).toContain('Timeline');
    rename({ title: 'Renamed' });
    expect(tab.element.textContent).toContain('Renamed');
    const x = tab.element.querySelector<HTMLElement>('.imt-tab-close')!;
    expect(x.getAttribute('aria-hidden')).toBe('true');
    x.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
    x.click();
    expect(close).toHaveBeenCalledOnce();
    tab.dispose();
    expect(dispose).toHaveBeenCalled();
    new PanelTab().dispose();
  });

  it('finds the panel id of the tab an event came from', () => {
    const host = document.createElement('div');
    host.innerHTML = '<div role="tab"><div class="imt-tab" data-panel-id="p1"><span id="inner"></span></div></div><div id="other"></div>';
    expect(tabPanelId(host.querySelector('#inner'))).toBe('p1');
    expect(tabPanelId(host.querySelector('#other'))).toBeNull();
    expect(tabPanelId(null)).toBeNull();
  });
});

describe('DockService with dockview', () => {
  let container: HTMLElement;
  let saved: SerializedDockview | null;
  let layoutStore: LayoutStore & { saves: number; cleared: number };

  function setup(opts: { plugin?: boolean; query?: boolean; stored?: SerializedDockview | null; backend?: MockBackend } = {}) {
    saved = opts.stored ?? null;
    layoutStore = {
      saves: 0,
      cleared: 0,
      load: () => (saved ? { layout: saved, extras: EMPTY_EXTRAS } : null),
      save: (l) => {
        saved = l.layout;
        layoutStore.saves++;
      },
      clear: () => {
        saved = null;
        layoutStore.cleared++;
      },
    };
    TestBed.configureTestingModule({
      providers: [provideDockTesting(opts.backend), { provide: LAYOUT_STORE, useValue: layoutStore }, ...(opts.plugin ? [provideImtPlugin(REGISTRY_PLUGIN)] : []), ...(opts.query ? [provideImtPlugin(QUERY_PLUGIN)] : [])],
    });
    container = document.createElement('div');
    document.body.appendChild(container);
    return { dock: TestBed.inject(DockService), store: TestBed.inject(WorkspaceStore) };
  }

  afterEach(() => {
    TestBed.inject(DockService).detach();
    container.remove();
  });

  const ids = (d: DockService) => [...d.openIds()].sort();
  const persisted = () => new Promise((r) => setTimeout(r, 300));

  /** The fuller arrangement most feature tests exercise: the inspector docked on the right, metrics tabbed with the timeline. */
  const withPanels = (d: DockService) => {
    d.pin(PANEL.inspector);
    d.show(PANEL.metrics, { inactive: true });
    return d;
  };

  it('builds the minimal default arrangement: timeline over events, the inspector auto-hidden on the right (metrics and the rest open on demand)', () => {
    const { dock } = setup();
    dock.attach(container);
    expect(ids(dock)).toEqual(['events', 'inspector', 'timeline']);
    expect(dock.isOpen('metrics')).toBe(false);
    const groups = [...container.querySelectorAll('.dv-groupview')];
    expect(groups).toHaveLength(2);
    const tabsOf = (g: Element) => [...g.querySelectorAll('[role=tab]')].map((t) => t.getAttribute('aria-label'));
    expect(groups.map(tabsOf).sort()).toEqual([['Events'], ['Timeline']].sort());
    expect(dock['api']!.getPanel('inspector')).toBeUndefined();
    expect(dock.autoHide()).toMatchObject({ items: [{ id: 'inspector', side: 'right', slot: { referencePanel: null } }], open: null });
    expect(container.querySelector('[role=tab][aria-selected=true]')).not.toBeNull();
  });

  it('slides the inspector back in when another tool window is shown (it would cover it), not for documents', async () => {
    const { dock } = setup({ query: true });
    dock.attach(container);
    await settle();
    dock.show('inspector');
    dock.show('profile');
    expect(dock.autoHide().open).toBe('inspector');
    dock.show('test-tool', { inactive: true });
    expect(dock.autoHide().open).toBe('inspector');
    dock.close('test-tool');
    dock.show('test-tool');
    expect(dock.autoHide().open).toBeNull();
    expect(dock.isOpen('test-tool')).toBe(true);
  });

  it('does not slide the inspector out over a tool window docked on its edge', async () => {
    const { dock, store } = setup({ query: true });
    dock.attach(container);
    await settle();
    dock.show('test-tool');
    store.select({ id: 4 } as never);
    await settle();
    expect(dock.autoHide().open).toBeNull();
    dock.close('test-tool');
    store.select({ id: 5 } as never);
    await settle();
    expect(dock.autoHide().open).toBe('inspector');
  });

  it('pins the default auto-hidden inspector as a full-height tool window on the right', () => {
    const { dock } = setup();
    dock.attach(container);
    dock.pin('inspector');
    expect(dock.autoHide().items).toHaveLength(0);
    const api = dock['api']!;
    expect(api.getPanel('inspector')!.group).not.toBe(api.getPanel('timeline')!.group);
    expect(container.querySelectorAll('.dv-groupview')).toHaveLength(3);
  });

  it('slides the auto-hidden inspector out when an event is selected, but not for the same one or a cleared selection', async () => {
    const { dock, store } = setup();
    dock.attach(container);
    await settle();
    const row = { id: 7 } as never;
    store.select(row);
    await settle();
    expect(dock.autoHide().open).toBe('inspector');
    dock.autoHideEvent({ type: 'blur' });
    store.select(row);
    await settle();
    expect(dock.autoHide().open).toBeNull();
    store.select(null);
    await settle();
    expect(dock.autoHide().open).toBeNull();
    store.select({ id: 8 } as never);
    await settle();
    expect(dock.autoHide().open).toBe('inspector');
    // Docked (pinned) or closed, selecting leaves the inspector alone; a pop-out window never slides it out.
    dock.pin('inspector');
    store.select({ id: 9 } as never);
    await settle();
    expect(dock.autoHide().open).toBeNull();
    dock.unpin('inspector');
    dock.setRemote(() => undefined);
    dock.revealInspector();
    expect(dock.autoHide().open).toBeNull();
    dock.setRemote(null);
    dock.close('inspector');
    dock.revealInspector();
    expect(dock.isOpen('inspector')).toBe(false);
  });

  it('opens, focuses, closes and toggles views; ignores unknown ones', () => {
    const { dock } = setup();
    dock.attach(container);
    dock.close(PANEL.metrics);
    expect(dock.isOpen('metrics')).toBe(false);
    dock.show('metrics');
    expect(dock.isOpen('metrics')).toBe(true);
    dock.show('nope');
    expect(dock.isOpen('nope')).toBe(false);
    dock.toggle('metrics');
    expect(dock.isOpen('metrics')).toBe(false);
    dock.toggle('metrics');
    expect(dock.isOpen('metrics')).toBe(true);
    dock.close('missing');
    dock.show('profile', { inactive: true });
    dock.show('timeline');
  });

  it('does nothing before attach or after detach', () => {
    const { dock } = setup();
    expect(() => {
      dock.show('timeline');
      dock.close('timeline');
      dock.reset();
      dock.toggleFloat({} as never);
    }).not.toThrow();
    dock.attach(container);
    dock.detach();
    expect(dock.openIds().size).toBe(0);
    expect(dock.autoHide().items).toHaveLength(0);
  });

  it('un-maximizes when showing a panel in another group', () => {
    const { dock } = setup();
    dock.attach(container);
    withPanels(dock);
    const maximize = container.querySelector<HTMLButtonElement>('button[aria-label="Maximize panel"]')!;
    maximize.click();
    expect(container.querySelector('button[aria-label="Restore panel size"]')).not.toBeNull();
    dock.show('inspector');
    expect(container.querySelector('button[aria-label="Restore panel size"]')).toBeNull();
  });

  it('floats a group and docks it back', () => {
    const { dock } = setup();
    dock.attach(container);
    withPanels(dock);
    container.querySelector<HTMLButtonElement>('button[aria-label="Float panel"]')!.click();
    expect(container.querySelector('.dv-resize-container')).not.toBeNull();
    const dockBack = container.querySelector<HTMLButtonElement>('button[aria-label="Dock panel back into the window"]')!;
    expect(dockBack).not.toBeNull();
    dockBack.click();
    expect(container.querySelector('button[aria-label="Dock panel back into the window"]')).toBeNull();
    expect(ids(dock)).toHaveLength(4);
  });

  it('docks a floated view back next to where it opens by default (below or right of the timeline)', () => {
    const { dock } = setup();
    dock.attach(container);
    withPanels(dock);
    const api = dock['api']!;
    for (const id of ['events', 'inspector']) {
      dock.toggleFloat(api.getPanel(id)!.group);
      expect(api.getPanel(id)!.group.api.location.type).toBe('floating');
      dock.toggleFloat(api.getPanel(id)!.group);
      expect(api.getPanel(id)!.group.api.location.type).toBe('grid');
      expect(container.querySelectorAll('.dv-groupview')).toHaveLength(3);
    }
  });

  it('docks a floating group even when it is the only group', () => {
    const { dock } = setup();
    dock.attach(container);
    for (const id of ['events', 'inspector', 'metrics', 'profile']) dock.close(id);
    container.querySelector<HTMLButtonElement>('button[aria-label="Float panel"]')!.click();
    container.querySelector<HTMLButtonElement>('button[aria-label="Dock panel back into the window"]')!.click();
    expect(ids(dock)).toEqual(['timeline']);
  });

  it('closes a focused tab with Delete and ignores other keys', () => {
    const { dock } = setup();
    dock.attach(container);
    withPanels(dock);
    const tab = [...container.querySelectorAll<HTMLElement>('[role=tab]')].find((t) => t.getAttribute('aria-label') === 'Metrics')!;
    tab.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(dock.isOpen('metrics')).toBe(true);
    tab.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true }));
    expect(dock.isOpen('metrics')).toBe(false);
    container.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true }));
  });

  it('persists the layout after changes (debounced) and restores it', async () => {
    const first = setup();
    first.dock.attach(container);
    withPanels(first.dock);
    first.dock.close('metrics');
    await persisted();
    expect(layoutStore.saves).toBeGreaterThan(0);
    expect(Object.keys(saved!.panels)).not.toContain('metrics');
    first.dock.detach();
    TestBed.resetTestingModule();
    container.remove();

    const second = setup({ stored: saved });
    second.dock.attach(container);
    expect(ids(second.dock)).toEqual(['events', 'inspector', 'timeline']);
  });

  it('falls back to the default layout when the saved one cannot be restored', () => {
    const { dock } = setup({ stored: { grid: null, panels: {} } as never });
    dock.attach(container);
    expect(layoutStore.cleared).toBe(1);
    expect(ids(dock)).toEqual(['events', 'inspector', 'timeline']);
    expect(dock.autoHide().items.map((i) => i.id)).toEqual(['inspector']);
    expect(testLogger().entries.some((e) => e.category === 'ui.layout' && e.level === 'warn')).toBe(true);
  });

  it('shows a placeholder for saved panels whose view no longer exists', () => {
    const first = setup();
    first.dock.attach(container);
    const layout = JSON.parse(JSON.stringify(first.dock['api']!.toJSON())) as SerializedDockview;
    first.dock.detach();
    must(layout.panels['timeline']).contentComponent = 'ghost';
    saved = layout;
    first.dock.attach(container);
    expect(container.textContent).toContain('"ghost" is not available');
  });

  it('pins the query bar in its own short group right above the events grid, and puts it back there', async () => {
    const { dock, store } = setup({ query: true });
    dock.attach(container);
    await settle();
    expect(dock.isOpen(QUERY_VIEW)).toBe(true);
    const order = () => groupsInOrder((dock as unknown as { api: { toJSON(): SerializedDockview } }).api.toJSON());
    expect(order()).toEqual([['timeline'], [QUERY_VIEW], ['events']]);
    // Gone while there is nothing to query; back above the grid (not tabbed with it) when there is.
    await store.clearWorkspace();
    await settle();
    expect(dock.isOpen(QUERY_VIEW)).toBe(false);
    await store.openPaths(['/x/a.log']);
    await settle();
    expect(order()).toEqual([['timeline'], [QUERY_VIEW], ['events']]);
    // With a real size (jsdom has no layout), the query group gets its short fixed height.
    const api = (dock as unknown as { api: DockviewApi }).api;
    api.layout(1200, 800);
    dock.reset();
    expect(order()).toEqual([['timeline'], [QUERY_VIEW], ['events']]);
    expect(api.getPanel(QUERY_VIEW)!.group.api.height).toBe(QUERY_HEIGHT);
    expect(api.getPanel('timeline')!.group.api.height).toBe(300);
  });

  it('opens the query bar by its default position when the events grid is closed', async () => {
    const { dock, store } = setup({ query: true });
    dock.attach(container);
    await settle();
    dock.close('events');
    dock.close(QUERY_VIEW);
    dock.show(QUERY_VIEW);
    expect(dock.isOpen(QUERY_VIEW)).toBe(true);
    void store;
  });

  it('reset restores the default arrangement and forgets the saved one', async () => {
    const { dock } = setup();
    dock.attach(container);
    withPanels(dock);
    dock.close('events');
    dock.reset();
    expect(ids(dock)).toEqual(['events', 'inspector', 'timeline']);
    expect(dock.autoHide().items.map((i) => i.id)).toEqual(['inspector']);
    expect(layoutStore.cleared).toBeGreaterThan(0);
    await persisted();
    expect(Object.keys(saved!.panels)).toContain('events');
  });

  it('reveals the events grid when a jump to a record is requested', async () => {
    const { dock, store } = setup();
    dock.attach(container);
    await settle();
    dock.close('events');
    store.jumpTo(3);
    await settle();
    expect(dock.isOpen('events')).toBe(true);
  });

  it('opens a conditional view on a fresh layout, closes it when it stops applying, and re-opens it when it applies again', async () => {
    const { dock, store } = setup({ plugin: true });
    dock.attach(container);
    await settle();
    expect(dock.isOpen('registry')).toBe(false); // the tiny dataset has no registry source
    await store.openPaths(['/x/export.reg']);
    expect(dock.isOpen('registry')).toBe(false); // the mock opens extra files as plain text
    // Make the summary contain a registry source.
    const summary = structuredClone(store.summary()!);
    summary.sources.push({ ...at(summary.sources, 0), id: 77, importer: 'registry', name: 'r.reg' });
    store.applySummary(summary);
    await settle();
    expect(dock.isOpen('registry')).toBe(true);
    dock.close('registry');
    store.applySummary(summary);
    await settle();
    expect(dock.isOpen('registry')).toBe(false); // does not nag after the user closed it
    dock.show('registry');
    expect(dock.isOpen('registry')).toBe(true);
    store.applySummary({ ...summary, sources: summary.sources.filter((s) => s.importer !== 'registry') });
    await settle();
    expect(dock.isOpen('registry')).toBe(false);
    dock.show('registry');
    expect(dock.isOpen('registry')).toBe(false); // unavailable views cannot be opened
  });

  it('does not open a conditional view on a restored layout, only when it newly applies', async () => {
    const first = setup();
    first.dock.attach(container);
    const layout = JSON.parse(JSON.stringify(first.dock['api']!.toJSON())) as SerializedDockview;
    first.dock.detach();
    TestBed.resetTestingModule();
    container.remove();

    const { dock, store } = setup({ plugin: true, stored: layout });
    dock.attach(container);
    await settle();
    const summary = structuredClone(store.summary()!);
    summary.sources.push({ ...at(summary.sources, 0), id: 77, importer: 'registry', name: 'r.reg' });
    store.applySummary(summary);
    await settle();
    expect(dock.isOpen('registry')).toBe(true);
  });

  it('keeps a restored layout without the conditional view when the workspace already had it', async () => {
    const backend = new MockBackend({ latency: false, dataset: registryDataset });
    const first = setup({ plugin: true, backend });
    first.dock.attach(container);
    await settle();
    first.dock.close('registry');
    const layout = JSON.parse(JSON.stringify(first.dock['api']!.toJSON())) as SerializedDockview;
    first.dock.detach();
    TestBed.resetTestingModule();
    container.remove();

    const second = setup({ plugin: true, stored: layout, backend: new MockBackend({ latency: false, dataset: registryDataset }) });
    second.dock.attach(container);
    await settle();
    expect(second.dock.isOpen('registry')).toBe(false);
  });
});

describe('DockHost', () => {
  it('attaches the dock after render and detaches on destroy', async () => {
    TestBed.configureTestingModule({ providers: [provideDockTesting(), { provide: LAYOUT_STORE, useValue: { load: () => null, save: () => undefined, clear: () => undefined } }] });
    const fixture = TestBed.createComponent(DockHost);
    await settle(fixture);
    const dock = TestBed.inject(DockService);
    expect(dock.isOpen('timeline')).toBe(true);
    fixture.destroy();
    expect(dock.isOpen('timeline')).toBe(false);
  });
});

describe('GroupActions', () => {
  it('only wires up once initialised and disposes its subscriptions', () => {
    const a = new GroupActions({ toggleFloat: () => undefined, popOut: () => undefined, autoHide: () => undefined, isTool: () => false, canPopOut: () => true, openMenu: () => undefined });
    a.dispose();
    expect(a.element.querySelectorAll('button')).toHaveLength(5);
  });
});

describe('keepGroupsUnscrolled', () => {
  it('puts a group scrolled sideways back, leaves vertical scrolling and other elements alone, and stops when disposed', () => {
    const root = document.createElement('div');
    const group = document.createElement('div');
    group.className = 'dv-groupview';
    const other = document.createElement('div');
    root.append(group, other);
    let left = 35;
    let otherLeft = 10;
    Object.defineProperty(group, 'scrollLeft', { get: () => left, set: (v: number) => (left = v) });
    Object.defineProperty(other, 'scrollLeft', { get: () => otherLeft, set: (v: number) => (otherLeft = v) });
    const watch = keepGroupsUnscrolled(root);
    group.dispatchEvent(new Event('scroll'));
    other.dispatchEvent(new Event('scroll'));
    expect(left).toBe(0);
    expect(otherLeft).toBe(10);
    group.dispatchEvent(new Event('scroll'));
    expect(left).toBe(0);
    watch.dispose();
    left = 20;
    group.dispatchEvent(new Event('scroll'));
    expect(left).toBe(20);
  });
});
