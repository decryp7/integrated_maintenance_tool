import { BrowserPopoutBackend, CLOSE_POLL_MS, POPOUT_CLOSED_EVENT, POPOUTS_CHANGED_EVENT, PopoutClosed, TauriPopoutBackend } from './popout-backend';
import { BroadcastChannelPort } from './sync-port';
import { at } from '../../testing/must';

describe('TauriPopoutBackend', () => {
  function make() {
    const call = vi.fn(async (..._a: unknown[]) => undefined);
    const handlers = new Map<string, (p: unknown) => void>();
    const unlisten = vi.fn();
    const listen = vi.fn(async (event: string, h: (p: unknown) => void) => {
      handlers.set(event, h);
      return unlisten;
    });
    const labelOf = vi.fn(async () => 'main');
    const backend = new TauriPopoutBackend(call as never, listen, labelOf);
    return { backend, call, listen, handlers, unlisten };
  }

  it('maps commands to invoke calls', async () => {
    const { backend, call } = make();
    const request = { viewId: 'v', title: 'T' };
    await backend.open(request);
    await backend.dock('p-main-1');
    await backend.list();
    await backend.setAlwaysOnTop(true);
    expect(call.mock.calls).toEqual([
      ['popout_panel', { request }],
      ['dock_panel', { popoutLabel: 'p-main-1' }],
      ['list_popouts'],
      ['set_popout_on_top', { on: true }],
    ]);
  });

  it('dockSelf docks the route label', async () => {
    const { backend, call } = make();
    await backend.dockSelf({ view: 'v', host: 'main', label: 'p-main-3' });
    expect(call).toHaveBeenCalledWith('dock_panel', { popoutLabel: 'p-main-3' });
  });

  it('propagates invoke failures', async () => {
    const { backend, call } = make();
    call.mockRejectedValueOnce(new Error('denied'));
    await expect(backend.list()).rejects.toThrow('denied');
  });

  it('reports its label and that the host is never gone', async () => {
    const { backend } = make();
    expect(await backend.label()).toBe('main');
    expect(backend.hostGone()).toBe(false);
  });

  it('onClosed forwards well-formed payloads and ignores malformed ones', async () => {
    const { backend, handlers, unlisten } = make();
    const got: PopoutClosed[] = [];
    const off = await backend.onClosed((e) => got.push(e));
    const h = handlers.get(POPOUT_CLOSED_EVENT)!;
    h({ label: 'p-1', viewId: 'v', extra: 1 });
    for (const bad of [null, undefined, 'x', {}, { label: 'p' }, { label: 1, viewId: 'v' }, { viewId: 'v' }]) h(bad);
    expect(got).toEqual([{ label: 'p-1', viewId: 'v' }]);
    expect(off).toBe(unlisten);
  });

  it('onChanged calls the handler without the payload', async () => {
    const { backend, handlers } = make();
    const fn = vi.fn();
    await backend.onChanged(fn);
    handlers.get(POPOUTS_CHANGED_EVENT)!({ ignored: true });
    expect(fn).toHaveBeenCalledWith();
  });
});

interface FakeChild {
  closed: boolean;
  focus: ReturnType<typeof vi.fn>;
  close: ReturnType<typeof vi.fn>;
  screenX: number;
  screenY: number;
  outerWidth: number;
  outerHeight: number;
}

describe('BrowserPopoutBackend', () => {
  const RANDOM = 0.5;
  const OWN = `b${Math.floor(RANDOM * 2 ** 32).toString(36)}`;

  function child(): FakeChild {
    const c: FakeChild = { closed: false, focus: vi.fn(), close: vi.fn(() => (c.closed = true)), screenX: 10, screenY: 20, outerWidth: 300, outerHeight: 200 };
    return c;
  }

  function make(overrides: Record<string, unknown> = {}) {
    const children: FakeChild[] = [];
    const win = {
      location: { pathname: '/app/', search: '?x=1', hash: '' },
      open: vi.fn((..._a: unknown[]) => {
        const c = child();
        children.push(c);
        return c;
      }),
      screen: { availWidth: 1600, availHeight: 900, availLeft: 5, availTop: 6 },
      opener: null as unknown,
      screenX: 1,
      screenY: 2,
      outerWidth: 1000,
      outerHeight: 700,
      close: vi.fn(),
      ...overrides,
    };
    const backend = new BrowserPopoutBackend(win as unknown as Window, () => RANDOM);
    return { backend, win, children };
  }

  afterEach(() => vi.useRealTimers());

  it('labels itself b<base36>, or with the route label of a pop-out', async () => {
    expect(await make().backend.label()).toBe(OWN);
    const { backend } = make({ location: { pathname: '/', search: '', hash: '#/popout?view=v&host=h&label=p-h-1' } });
    expect(await backend.label()).toBe('p-h-1');
  });

  it('opens a popup with the route url and bounds features', async () => {
    const { backend, win } = make();
    const info = await backend.open({ viewId: 'v', title: 'T', params: { a: 1 }, bounds: { x: 10.4, y: 20.6, width: 400, height: 300 } });
    const label = `p-${OWN}-1`;
    expect(win.open).toHaveBeenCalledWith(`/app/?x=1#/popout?view=v&host=${OWN}&label=${label}`, label, 'popup,left=10,top=21,width=400,height=300');
    expect(info).toEqual({ label, host: OWN, viewId: 'v', title: 'T', params: { a: 1 }, bounds: { x: 10.4, y: 20.6, width: 400, height: 300 }, monitor: null, alwaysOnTop: false });
  });

  it('uses default features and the view id as title without bounds', async () => {
    const { backend, win } = make();
    const info = await backend.open({ viewId: 'v' });
    expect(win.open.mock.calls[0]?.[2]).toBe('popup,width=560,height=420');
    expect(info.title).toBe('v');
    expect(info.bounds).toBeNull();
  });

  it('throws a helpful error when the popup is blocked', async () => {
    const { backend, win } = make();
    win.open.mockReturnValueOnce(null as never);
    await expect(backend.open({ viewId: 'v' })).rejects.toThrow(/allow pop-ups/);
    expect(await backend.list()).toEqual([]);
  });

  it('focuses the existing window when the same view is opened again', async () => {
    const { backend, win, children } = make();
    const a = await backend.open({ viewId: 'v' });
    const b = await backend.open({ viewId: 'v' });
    expect(b).toBe(a);
    expect(win.open).toHaveBeenCalledTimes(1);
    expect(children[0]?.focus).toHaveBeenCalled();
    const c = await backend.open({ viewId: 'other' });
    expect(c.label).toBe(`p-${OWN}-2`);
  });

  it('opens a new window when the previous one for the view was closed', async () => {
    const { backend, win, children } = make();
    await backend.open({ viewId: 'v' });
    at(children, 0).closed = true;
    await backend.open({ viewId: 'v' });
    expect(win.open).toHaveBeenCalledTimes(2);
  });

  it('lists open pop-outs with live bounds from the child window', async () => {
    const { backend, children } = make();
    await backend.open({ viewId: 'a' });
    await backend.open({ viewId: 'b' });
    at(children, 1).closed = true;
    const list = await backend.list();
    expect(list.map((p) => p.viewId)).toEqual(['a']);
    expect(list[0]?.bounds).toEqual({ x: 10, y: 20, width: 300, height: 200 });
  });

  it('keeps the stored bounds when the child window cannot be read', async () => {
    const { backend, children } = make();
    await backend.open({ viewId: 'a', bounds: { x: 1, y: 2, width: 3, height: 4 } });
    Object.defineProperty(children[0], 'screenX', {
      get() {
        throw new Error('cross-origin');
      },
    });
    expect((await backend.list())[0]?.bounds).toEqual({ x: 1, y: 2, width: 3, height: 4 });
  });

  it('dock closes the child and reports it closed', async () => {
    const { backend, children } = make();
    const closed: PopoutClosed[] = [];
    await backend.onClosed((e) => closed.push(e));
    const info = await backend.open({ viewId: 'v' });
    await backend.dock(info.label);
    expect(children[0]?.close).toHaveBeenCalled();
    expect(closed).toEqual([{ label: info.label, viewId: 'v' }]);
    expect(await backend.list()).toEqual([]);
  });

  it('dock of an unknown label rejects', async () => {
    await expect(make().backend.dock('nope')).rejects.toThrow(/already closed/);
  });

  it('notices popups closed by the user by polling, and stops polling when none remain', async () => {
    vi.useFakeTimers();
    const { backend, children } = make();
    const closed: PopoutClosed[] = [];
    await backend.onClosed((e) => closed.push(e));
    const info = await backend.open({ viewId: 'v' });
    at(children, 0).closed = true;
    vi.advanceTimersByTime(CLOSE_POLL_MS);
    expect(closed).toEqual([{ label: info.label, viewId: 'v' }]);
    expect(vi.getTimerCount()).toBe(0);
    vi.advanceTimersByTime(CLOSE_POLL_MS * 3);
    expect(closed).toHaveLength(1);
  });

  it('onChanged fires on open and on close and can be unsubscribed', async () => {
    const { backend, children } = make();
    const fn = vi.fn();
    const off = await backend.onChanged(fn);
    await backend.open({ viewId: 'v' });
    expect(fn).toHaveBeenCalledTimes(1);
    at(children, 0).closed = true;
    backend.poll();
    expect(fn).toHaveBeenCalledTimes(2);
    off();
    await backend.open({ viewId: 'w' });
    expect(fn).toHaveBeenCalledTimes(2);
  });

  it('stops notifying an unsubscribed closed handler', async () => {
    const { backend, children } = make();
    const fn = vi.fn();
    const off = await backend.onClosed(fn);
    await backend.open({ viewId: 'v' });
    off();
    at(children, 0).closed = true;
    backend.poll();
    expect(fn).not.toHaveBeenCalled();
  });

  it('reports work areas from the screen, none without screen info', async () => {
    expect(await make().backend.workAreas()).toEqual([{ name: null, primary: true, x: 5, y: 6, width: 1600, height: 900 }]);
    expect(await make({ screen: { availWidth: 800, availHeight: 600 } }).backend.workAreas()).toEqual([{ name: null, primary: true, x: 0, y: 0, width: 800, height: 600 }]);
    expect(await make({ screen: {} }).backend.workAreas()).toEqual([]);
    expect(await make({ screen: undefined }).backend.workAreas()).toEqual([]);
  });

  it('reports its own window bounds', async () => {
    expect(await make().backend.windowBounds()).toEqual({ x: 1, y: 2, width: 1000, height: 700 });
  });

  it('dockSelf closes the window; setAlwaysOnTop is a no-op', async () => {
    const { backend, win } = make();
    await backend.dockSelf();
    expect(win.close).toHaveBeenCalled();
    await expect(backend.setAlwaysOnTop()).resolves.toBeUndefined();
  });

  it('hostGone is true without an opener or with a closed one', () => {
    expect(make({ opener: null }).backend.hostGone()).toBe(true);
    expect(make({ opener: { closed: true } }).backend.hostGone()).toBe(true);
    expect(make({ opener: { closed: false } }).backend.hostGone()).toBe(false);
  });

  it('syncPort returns a BroadcastChannelPort', () => {
    const orig = globalThis.BroadcastChannel;
    globalThis.BroadcastChannel = class {
      postMessage() {}
      addEventListener() {}
      removeEventListener() {}
      close() {}
    } as unknown as typeof BroadcastChannel;
    try {
      const port = make().backend.syncPort('g');
      expect(port).toBeInstanceOf(BroadcastChannelPort);
      expect(port.needsRelay).toBe(false);
    } finally {
      globalThis.BroadcastChannel = orig;
    }
  });
});

describe('pop-out moves and window drags', () => {
  it('parses moved payloads and rejects malformed ones', async () => {
    const { parseMoved, TauriPopoutBackend } = await import('./popout-backend');
    expect(parseMoved({ label: 'p-main-1', viewId: 'inspector', x: 1, y: 2 })).toEqual({ label: 'p-main-1', viewId: 'inspector', x: 1, y: 2 });
    for (const bad of [null, {}, { label: 'p', viewId: 'v', x: 'a', y: 1 }, { label: 'p', viewId: 'v', x: 1, y: Infinity }]) expect(parseMoved(bad)).toBeNull();
    let handler: (p: unknown) => void = () => undefined;
    const seen: unknown[] = [];
    const b = new TauriPopoutBackend(vi.fn() as never, async (_e, h) => ((handler = h), () => undefined), async () => 'main');
    await b.onMoved((e) => seen.push(e));
    handler({ label: 'p-main-1', viewId: 'v', x: 3, y: 4 });
    handler({ nope: 1 });
    expect(seen).toEqual([{ label: 'p-main-1', viewId: 'v', x: 3, y: 4 }]);
    expect(b.supportsAlwaysOnTop).toBe(true);
  });

  it('in a browser: no OS window drag, no always-on-top, and the content origin from the window metrics', async () => {
    const { BrowserPopoutBackend } = await import('./popout-backend');
    const win = { screenX: 100, screenY: 50, outerWidth: 1010, innerWidth: 1000, outerHeight: 840, innerHeight: 800, location: { hash: '', pathname: '/', search: '' } } as unknown as Window;
    const b = new BrowserPopoutBackend(win, () => 0.5);
    expect(await b.startWindowDrag()).toBe(false);
    expect(b.supportsAlwaysOnTop).toBe(false);
    expect(await b.contentOrigin()).toEqual({ x: 105, y: 90 });
    const off = await b.onMoved();
    expect(() => off()).not.toThrow();
  });
});

describe('contentOriginOf', () => {
  it('finds the content below the title bar and inside the borders, in logical px', async () => {
    const { contentOriginOf } = await import('./popout-backend');
    // macOS: no side borders, a 28 px title bar.
    expect(contentOriginOf({ x: 1920, y: 30 }, { width: 1920, height: 1078 }, { width: 1920, height: 1050 }, 1)).toEqual({ x: 1920, y: 58 });
    // Windows: 8 px resize borders, 31 px caption, at 150 %.
    expect(contentOriginOf({ x: 300, y: 150 }, { width: 1516, height: 939 }, { width: 1500, height: 900 }, 1.5)).toEqual({ x: 205.33333333333334, y: 120.66666666666667 });
    expect(contentOriginOf({ x: 10, y: 10 }, { width: 100, height: 100 }, { width: 100, height: 100 }, 0)).toEqual({ x: 10, y: 10 });
  });
});
