import { MOCK_ON_CLOSE_KEY, MockWindowHost, MockWindowingBackend, seededOpenAsk, seededTasks } from './mock-windowing-backend';
import { at } from '../../testing/must';

class FakeStorage {
  readonly map = new Map<string, string>();
  getItem = (k: string) => this.map.get(k) ?? null;
  setItem = (k: string, v: string) => void this.map.set(k, v);
}

function host(href = 'http://localhost:4200/?mockTotal=5', storage: Storage | null = new FakeStorage() as unknown as Storage) {
  const popups: { url: string; win: { closed: boolean; focus: ReturnType<typeof vi.fn> } }[] = [];
  const h: MockWindowHost & { popups: typeof popups; closed: number; titleText: string } = {
    popups,
    closed: 0,
    titleText: '',
    href: () => href,
    open: (url) => {
      const win = { closed: false, focus: vi.fn() };
      popups.push({ url, win });
      return win as unknown as Window;
    },
    close: () => void h.closed++,
    setTitle: (t) => void (h.titleText = t),
    title: () => h.titleText,
    hasFocus: () => true,
    storage: () => storage,
  };
  return h;
}

describe('MockWindowingBackend', () => {
  it('takes its label from the URL, defaulting to main', async () => {
    expect(await new MockWindowingBackend(host()).label()).toBe('main');
    expect(await new MockWindowingBackend(host('http://x/?window=w-3')).label()).toBe('w-3');
  });

  it('opens new windows as tabs of the same URL, keeping the query, and lists them until closed', async () => {
    const h = host();
    const b = new MockWindowingBackend(h);
    const changed = vi.fn();
    await b.onWindowsChanged(changed);
    const first = await b.newWindow();
    const second = await b.newWindow();
    expect(first.label).toBe('w-2');
    expect(second.label).toBe('w-3');
    const url = new URL(at(h.popups, 0).url);
    expect(url.searchParams.get('mockTotal')).toBe('5');
    expect(url.searchParams.get('window')).toBe('w-2');
    expect(changed).toHaveBeenCalledTimes(2);
    h.titleText = 'P — IMT';
    expect((await b.listWindows()).map((w) => [w.label, w.title])).toEqual([
      ['main', 'P — IMT'],
      ['w-2', 'IMT — w-2'],
      ['w-3', 'IMT — w-3'],
    ]);
    at(h.popups, 0).win.closed = true;
    expect((await b.listWindows()).map((w) => w.label)).toEqual(['main', 'w-3']);
    await b.focusWindow('w-3');
    await b.focusWindow('gone');
    expect(h.popups[1]?.win.focus).toHaveBeenCalled();
  });

  it('works without storage', async () => {
    const h = host(undefined, null);
    h.titleText = '';
    const b = new MockWindowingBackend(h);
    expect((await b.newWindow()).label).toBe('w-2');
    expect((await b.listWindows())[0]?.title).toBe('IMT');
    await b.resolveClose('tray', true);
    const handler = vi.fn();
    await b.onCloseRequested(handler);
    await b.closeWindow();
    expect(handler).toHaveBeenCalled();
  });

  it('survives storage that throws', async () => {
    const throwing = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('denied'); } } as unknown as Storage;
    const b = new MockWindowingBackend(host(undefined, throwing));
    expect((await b.newWindow()).label).toBe('w-2');
    await b.resolveClose('quit', true);
    const handler = vi.fn();
    await b.onCloseRequested(handler);
    await b.closeWindow();
    expect(handler).toHaveBeenCalled();
  });

  it('asks on close by default, then follows the remembered choice', async () => {
    const h = host();
    const b = new MockWindowingBackend(h);
    const asked = vi.fn();
    const unlisten = await b.onCloseRequested(asked);
    await b.closeWindow();
    expect(asked).toHaveBeenCalledTimes(1);
    await b.resolveClose('tray', false);
    await b.closeWindow();
    expect(asked).toHaveBeenCalledTimes(2);
    await b.resolveClose('tray', true);
    const info = vi.spyOn(console, 'info').mockImplementation(() => undefined);
    await b.closeWindow();
    // End-to-end tests wait for this line to know the close was handled.
    expect(info).toHaveBeenCalledWith('[mock windowing] close_window tray');
    info.mockRestore();
    expect(asked).toHaveBeenCalledTimes(2);
    expect(h.closed).toBe(0);
    await b.resolveClose('quit', true);
    expect(h.closed).toBe(1);
    await b.closeWindow();
    expect(h.closed).toBe(2);
    expect((h.storage() as Storage).getItem(MOCK_ON_CLOSE_KEY)).toBe('quit');
    expect(b.calls).toContain('resolveClose:tray:true');
    unlisten();
    expect(b.listeners).toBe(0);
  });

  it('has no background tasks unless the URL asks for them', async () => {
    expect(await new MockWindowingBackend(host()).backgroundTasks()).toEqual([]);
    const b = new MockWindowingBackend(host('http://x/?mock-tasks'));
    expect((await b.backgroundTasks()).map((t) => t.id)).toEqual(seededTasks().map((t) => t.id));
  });

  it('asks where to open seeded files once the UI listens, and only once', async () => {
    expect(seededOpenAsk(null)).toBeNull();
    expect(seededOpenAsk(' , ')).toBeNull();
    expect(seededOpenAsk('a.log, b.log')).toEqual({ count: 2, names: ['a.log', 'b.log'] });
    expect(seededOpenAsk(Array.from({ length: 30 }, (_, i) => `f${i}`).join(','))?.count).toBe(20);
    vi.useFakeTimers();
    try {
      const b = new MockWindowingBackend(host('http://x/?mock-open-ask=a.log'));
      const first = vi.fn();
      const second = vi.fn();
      await b.onOpenAsk(first);
      await b.onOpenAsk(second);
      vi.runAllTimers();
      expect(first).toHaveBeenCalledWith({ count: 1, names: ['a.log'] });
      expect(first).toHaveBeenCalledTimes(1);
      expect(second).toHaveBeenCalledTimes(1);
      const info = vi.spyOn(console, 'info').mockImplementation(() => undefined);
      await b.resolveOpen('cancel');
      expect(info).toHaveBeenCalledWith('[mock windowing] resolve_open cancel');
      info.mockRestore();
    } finally {
      vi.useRealTimers();
    }
  });

  it('stops a task, notifies subscribers, and turns live tail off for its own window', async () => {
    const b = new MockWindowingBackend(host('http://x/?mock-tasks'));
    const tasks = vi.fn();
    const live = vi.fn();
    await b.onBackgroundTasks(tasks);
    await b.onLiveStopped(live);
    await b.stopBackgroundTask('monitor');
    expect(live).not.toHaveBeenCalled();
    await b.stopBackgroundTask('live-main');
    expect(live).toHaveBeenCalledTimes(1);
    expect(tasks.mock.calls.at(-1)![0].map((t: { id: string }) => t.id)).toEqual(['symbols']);
    await expect(b.stopBackgroundTask('symbols')).rejects.toContain('cannot be stopped');
    await expect(b.stopBackgroundTask('nope')).rejects.toContain('no background task');
  });

  it('records the other commands and emits simulated events', async () => {
    const h = host();
    const b = new MockWindowingBackend(h);
    await b.setWindowTitle('X — IMT');
    expect(h.titleText).toBe('X — IMT');
    expect(await b.restoreWindows()).toBe(0);
    await b.resolveOpen('newWindow');
    const ask = vi.fn();
    const capture = vi.fn();
    const live = vi.fn();
    await b.onOpenAsk(ask);
    await b.onCapture(capture);
    await b.onLiveStopped(live);
    b.emitOpenAsk({ count: 1, names: ['a.log'] });
    b.emitCapture();
    b.emitLiveStopped();
    expect(ask).toHaveBeenCalledWith({ count: 1, names: ['a.log'] });
    expect(capture).toHaveBeenCalled();
    expect(live).toHaveBeenCalled();
    expect(b.calls).toEqual(['restoreWindows', 'resolveOpen:newWindow']);
  });
});
