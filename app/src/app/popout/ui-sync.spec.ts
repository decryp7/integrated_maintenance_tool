import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, testLogger } from '../../testing/helpers';
import { Bus } from './popout-testing';
import { SyncMessage } from './sync-protocol';
import { HELLO_INTERVAL_MS, HELLO_TRIES, SyncedValue, UiSync } from './ui-sync';

function value<T>(key: string, initial: T, direction?: 'both' | 'fromHost') {
  const s = signal<T>(initial);
  const v: SyncedValue<T> = { key, read: () => s(), apply: (x) => s.set(x as T), ...(direction ? { direction } : {}) };
  return { s, v };
}

describe('UiSync', () => {
  let bus: Bus;
  beforeEach(() => {
    TestBed.configureTestingModule({ providers: provideTinyBackend() });
    bus = new Bus();
  });

  const make = () => TestBed.runInInjectionContext(() => new UiSync());
  const flush = () => TestBed.tick();

  function pair() {
    const host = { q: value('q', 'host'), d: value('data', 1, 'fromHost') };
    const pop = { q: value('q', 'default'), d: value('data', 0, 'fromHost') };
    const h = make();
    const p = make();
    h.start(bus.port(), true, 'main', [host.q.v, host.d.v], 'oh');
    const synced = vi.fn();
    p.onSynced(synced);
    p.start(bus.port(), false, 'p-main-1', [pop.q.v, pop.d.v], 'op');
    flush();
    return { h, p, host, pop, synced };
  }

  it('a pop-out takes the host snapshot on hello and does not push its defaults', () => {
    const { pop, synced, h, p } = pair();
    expect(pop.q.s()).toBe('host');
    expect(pop.d.s()).toBe(1);
    expect(synced).toHaveBeenCalledTimes(1);
    expect(bus.log.map((m) => m.t)).toEqual(['hello', 'snapshot']);
    expect(h.started && h.isHost && p.started && !p.isHost).toBe(true);
  });

  it('changes go both ways and are never echoed back', () => {
    const { host, pop } = pair();
    host.q.s.set('a');
    flush();
    expect(pop.q.s()).toBe('a');
    flush();
    expect(bus.log.filter((m) => m.t === 'state')).toHaveLength(1);
    pop.q.s.set('b');
    flush();
    expect(host.q.s()).toBe('b');
    flush();
    expect(bus.log.filter((m) => m.t === 'state')).toHaveLength(2);
  });

  it('host-only values flow from the host only', () => {
    const { host, pop } = pair();
    pop.d.s.set(99);
    flush();
    expect(host.d.s()).toBe(1);
    host.d.s.set(2);
    flush();
    expect(pop.d.s()).toBe(2);
    // A host-only value from a pop-out is ignored even when sent.
    bus.ports[0].handlers.forEach((fn) => fn({ t: 'state', origin: 'x', seq: 1, host: false, key: 'data', value: 7 }));
    expect(host.d.s()).toBe(2);
  });

  it('drops stale, duplicate, malformed and unknown messages', () => {
    const { host } = pair();
    const deliver = (m: unknown) => bus.ports[0].handlers.forEach((fn) => fn(m));
    deliver({ t: 'state', origin: 'z', seq: 5, host: false, key: 'q', value: 'new' });
    deliver({ t: 'state', origin: 'z', seq: 4, host: false, key: 'q', value: 'old' });
    deliver({ t: 'state', origin: 'z', seq: 5, host: false, key: 'q', value: 'dup' });
    deliver({ nonsense: true });
    deliver({ t: 'state', origin: 'z', seq: 6, host: false, key: 'unknown', value: 1 });
    expect(host.q.s()).toBe('new');
  });

  it('runs pop-out commands in the host and tells pop-outs when the host leaves', () => {
    const { h, p } = pair();
    const commands: string[] = [];
    h.onCommand((c, a) => commands.push(`${c}:${a}`));
    expect(p.runInHost('run', 'edit.undo')).toBe(true);
    expect(p.runInHost('showView')).toBe(true);
    expect(h.runInHost('run', 'x')).toBe(false);
    expect(commands).toEqual(['run:edit.undo', 'showView:undefined']);
    const gone = vi.fn();
    p.onHostGone(gone);
    p.goodbye();
    h.goodbye();
    expect(gone).toHaveBeenCalledTimes(1);
  });

  it('relays pop-out changes to the other pop-outs when the transport only reaches the host', () => {
    const host = value('q', 'h');
    const a = value('q', '');
    const h = make();
    h.start(bus.port(true), true, 'main', [host.v], 'oh');
    const relayed: SyncMessage[] = [];
    const pa = make();
    pa.start(bus.port(), false, 'p-main-1', [a.v], 'oa');
    flush();
    bus.ports[0].post = ((orig) => (m: SyncMessage) => {
      relayed.push(m);
      orig.call(bus.ports[0], m);
    })(bus.ports[0].post);
    a.s.set('from a');
    flush();
    expect(host.s()).toBe('from a');
    expect(relayed.some((m) => m.t === 'state' && m.origin === 'oa')).toBe(true);
  });

  it('contains a failing apply and refuses oversized messages', () => {
    const bad: SyncedValue = { key: 'q', read: () => 'x', apply: () => { throw new Error('boom'); } };
    const big = value('big', '');
    const h = make();
    h.start(bus.port(), true, 'main', [bad, big.v], 'oh');
    bus.ports[0].handlers.forEach((fn) => fn({ t: 'state', origin: 'z', seq: 1, host: false, key: 'q', value: 'y' }));
    expect(testLogger().entries.some((e) => e.message.includes('could not be applied'))).toBe(true);
    big.s.set('x'.repeat(600 * 1024));
    flush();
    expect(bus.log).toHaveLength(0);
    expect(testLogger().entries.some((e) => e.message.includes('too large'))).toBe(true);
    h.stop();
    expect(bus.ports[0].closed).toBe(true);
  });

  it('repeats hello until the host answers, at most a few times', () => {
    vi.useFakeTimers();
    try {
      const p = make();
      p.start(bus.port(), false, 'p-main-1', [], 'op');
      vi.advanceTimersByTime(HELLO_INTERVAL_MS * (HELLO_TRIES + 3));
      expect(bus.log.filter((m) => m.t === 'hello')).toHaveLength(HELLO_TRIES);
      // A snapshot for someone else is not ours.
      bus.port().post({ t: 'snapshot', origin: 'oh', seq: 1, host: true, to: 'p-main-9', values: {} });
      p.stop();
    } finally {
      vi.useRealTimers();
    }
  });
});
