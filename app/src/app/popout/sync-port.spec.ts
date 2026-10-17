import { BroadcastChannelPort, TauriEventPort, UI_SYNC_EVENT } from './sync-port';
import { SyncMessage } from './sync-protocol';

const msg: SyncMessage = { t: 'bye', origin: 'o', seq: 1, host: false };

class FakeChannel {
  listeners = new Set<(e: MessageEvent) => void>();
  posted: unknown[] = [];
  closed = false;
  throwOnPost = false;
  constructor(readonly name: string) {}
  postMessage(m: unknown) {
    if (this.throwOnPost) throw new Error('boom');
    this.posted.push(m);
  }
  addEventListener(_t: string, fn: (e: MessageEvent) => void) {
    this.listeners.add(fn);
  }
  removeEventListener(_t: string, fn: (e: MessageEvent) => void) {
    this.listeners.delete(fn);
  }
  close() {
    this.closed = true;
  }
  deliver(data: unknown) {
    for (const l of [...this.listeners]) l({ data } as MessageEvent);
  }
}

describe('BroadcastChannelPort', () => {
  function make() {
    let ch!: FakeChannel;
    const port = new BroadcastChannelPort('g1', (name) => (ch = new FakeChannel(name)) as unknown as BroadcastChannel);
    return { port, ch };
  }

  it('uses a channel named after the group and needs no relay', () => {
    const { port, ch } = make();
    expect(ch.name).toBe('imt-ui-sync:g1');
    expect(port.needsRelay).toBe(false);
  });

  it('posts messages', () => {
    const { port, ch } = make();
    port.post(msg);
    expect(ch.posted).toEqual([msg]);
  });

  it('swallows a throwing postMessage', () => {
    const { port, ch } = make();
    ch.throwOnPost = true;
    expect(() => port.post(msg)).not.toThrow();
  });

  it('listens and unsubscribes', () => {
    const { port, ch } = make();
    const got: unknown[] = [];
    const off = port.listen((m) => got.push(m));
    ch.deliver({ a: 1 });
    off();
    ch.deliver({ a: 2 });
    expect(got).toEqual([{ a: 1 }]);
    expect(ch.listeners.size).toBe(0);
  });

  it('closes the channel', () => {
    const { port, ch } = make();
    port.close();
    expect(ch.closed).toBe(true);
  });

  it('creates a real BroadcastChannel by default', () => {
    const created: string[] = [];
    const orig = globalThis.BroadcastChannel;
    globalThis.BroadcastChannel = class {
      constructor(n: string) {
        created.push(n);
      }
    } as unknown as typeof BroadcastChannel;
    try {
      new BroadcastChannelPort('x');
    } finally {
      globalThis.BroadcastChannel = orig;
    }
    expect(created).toEqual(['imt-ui-sync:x']);
  });
});

describe('TauriEventPort', () => {
  function make(isHost = true, targets: string[] = ['w1', 'w2']) {
    const emitted: [string, string, unknown][] = [];
    const emitTo = vi.fn(async (l: string, e: string, p: unknown) => {
      emitted.push([l, e, p]);
    });
    const off = vi.fn();
    let handler: ((p: unknown) => void) | null = null;
    let resolveListen!: () => void;
    const listenFn = vi.fn((_e: string, h: (p: unknown) => void) => {
      handler = h;
      return new Promise<() => void>((res) => (resolveListen = () => res(off)));
    });
    const port = new TauriEventPort(() => targets, emitTo, listenFn, isHost);
    return { port, emitted, emitTo, off, listenFn, resolveListen: () => resolveListen(), handler: () => handler };
  }
  const flush = () => new Promise((r) => setTimeout(r));

  it('posts to every target label with the sync event', () => {
    const { port, emitted } = make();
    port.post(msg);
    expect(emitted).toEqual([
      ['w1', UI_SYNC_EVENT, msg],
      ['w2', UI_SYNC_EVENT, msg],
    ]);
  });

  it('posts nothing without targets', () => {
    const { port, emitTo } = make(true, []);
    port.post(msg);
    expect(emitTo).not.toHaveBeenCalled();
  });

  it('ignores emit failures', async () => {
    const emitTo = vi.fn(() => Promise.reject(new Error('gone')));
    const port = new TauriEventPort(() => ['w'], emitTo, vi.fn(), false);
    expect(() => port.post(msg)).not.toThrow();
    await flush();
  });

  it('needsRelay reflects isHost', () => {
    expect(make(true).port.needsRelay).toBe(true);
    expect(make(false).port.needsRelay).toBe(false);
  });

  it('listens for the sync event and passes payloads on', async () => {
    const t = make();
    const got: unknown[] = [];
    t.port.listen((p) => got.push(p));
    expect(t.listenFn).toHaveBeenCalledWith(UI_SYNC_EVENT, expect.any(Function));
    t.handler()?.('payload');
    expect(got).toEqual(['payload']);
  });

  it('unsubscribe calls the underlying unlisten once it is known', async () => {
    const t = make();
    const stop = t.port.listen(() => undefined);
    t.resolveListen();
    await flush();
    stop();
    expect(t.off).toHaveBeenCalledTimes(1);
  });

  it('unlistens even when unsubscribed before the listen promise resolves', async () => {
    const t = make();
    const stop = t.port.listen(() => undefined);
    stop();
    expect(t.off).not.toHaveBeenCalled();
    t.resolveListen();
    await flush();
    expect(t.off).toHaveBeenCalledTimes(1);
  });

  it('survives a failing listen', async () => {
    const port = new TauriEventPort(() => [], vi.fn(), () => Promise.reject(new Error('no')), true);
    const stop = port.listen(() => undefined);
    await flush();
    expect(() => stop()).not.toThrow();
  });

  it('close stops posting and unlistens', async () => {
    const t = make();
    t.port.listen(() => undefined);
    t.resolveListen();
    await flush();
    t.port.close();
    expect(t.off).toHaveBeenCalledTimes(1);
    t.port.post(msg);
    expect(t.emitTo).not.toHaveBeenCalled();
  });
});
