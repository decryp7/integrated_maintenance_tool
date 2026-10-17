import { fingerprint, newOrigin, parseSyncMessage, SyncEngine, SyncMessage } from './sync-protocol';

const base = { origin: 'o1', seq: 1, host: true };

describe('parseSyncMessage', () => {
  it('parses each message type', () => {
    expect(parseSyncMessage({ t: 'state', ...base, key: 'k', value: { a: 1 } })).toEqual({ t: 'state', ...base, key: 'k', value: { a: 1 } });
    expect(parseSyncMessage({ t: 'hello', ...base, label: 'p-1' })).toEqual({ t: 'hello', ...base, label: 'p-1' });
    expect(parseSyncMessage({ t: 'snapshot', ...base, to: 'o2', values: { k: 1 } })).toEqual({ t: 'snapshot', ...base, to: 'o2', values: { k: 1 } });
    expect(parseSyncMessage({ t: 'command', ...base, command: 'go', arg: 'x' })).toEqual({ t: 'command', ...base, command: 'go', arg: 'x' });
    expect(parseSyncMessage({ t: 'command', ...base, command: 'go' })).toEqual({ t: 'command', ...base, command: 'go' });
    expect(parseSyncMessage({ t: 'bye', ...base })).toEqual({ t: 'bye', ...base });
  });

  it('accepts a state message whose value is null or undefined-but-present', () => {
    expect(parseSyncMessage({ t: 'state', ...base, key: 'k', value: null })).not.toBeNull();
    expect(parseSyncMessage({ t: 'state', ...base, key: 'k', value: undefined })).not.toBeNull();
  });

  it('strips unknown extra fields', () => {
    expect(parseSyncMessage({ t: 'bye', ...base, evil: 1 })).toEqual({ t: 'bye', ...base });
  });

  it.each([null, undefined, 'str', 5, true, [], [{ t: 'bye' }]])('rejects non-object %j', (raw) => {
    expect(parseSyncMessage(raw)).toBeNull();
  });

  it('rejects unknown or non-string types', () => {
    expect(parseSyncMessage({ t: 'nope', ...base })).toBeNull();
    expect(parseSyncMessage({ t: 5, ...base })).toBeNull();
    expect(parseSyncMessage({ ...base })).toBeNull();
  });

  it('rejects bad origins', () => {
    expect(parseSyncMessage({ t: 'bye', ...base, origin: '' })).toBeNull();
    expect(parseSyncMessage({ t: 'bye', ...base, origin: 'x'.repeat(65) })).toBeNull();
    expect(parseSyncMessage({ t: 'bye', ...base, origin: 1 })).toBeNull();
    expect(parseSyncMessage({ t: 'bye', ...base, origin: 'x'.repeat(64) })).not.toBeNull();
  });

  it('rejects bad sequence numbers', () => {
    for (const seq of [-1, 1.5, '1', NaN, Infinity, Number.MAX_SAFE_INTEGER + 2, undefined]) {
      expect(parseSyncMessage({ t: 'bye', ...base, seq })).toBeNull();
    }
    expect(parseSyncMessage({ t: 'bye', ...base, seq: 0 })).not.toBeNull();
  });

  it('requires a boolean host flag', () => {
    expect(parseSyncMessage({ t: 'bye', origin: 'o1', seq: 1 })).toBeNull();
    expect(parseSyncMessage({ t: 'bye', origin: 'o1', seq: 1, host: 'true' })).toBeNull();
  });

  it('rejects missing or mistyped type-specific fields', () => {
    expect(parseSyncMessage({ t: 'state', ...base, value: 1 })).toBeNull();
    expect(parseSyncMessage({ t: 'state', ...base, key: 'k' })).toBeNull();
    expect(parseSyncMessage({ t: 'state', ...base, key: 3, value: 1 })).toBeNull();
    expect(parseSyncMessage({ t: 'hello', ...base })).toBeNull();
    expect(parseSyncMessage({ t: 'hello', ...base, label: 1 })).toBeNull();
    expect(parseSyncMessage({ t: 'snapshot', ...base, values: {} })).toBeNull();
    expect(parseSyncMessage({ t: 'snapshot', ...base, to: 'x' })).toBeNull();
    expect(parseSyncMessage({ t: 'snapshot', ...base, to: 'x', values: [] })).toBeNull();
    expect(parseSyncMessage({ t: 'snapshot', ...base, to: 'x', values: null })).toBeNull();
    expect(parseSyncMessage({ t: 'command', ...base })).toBeNull();
    expect(parseSyncMessage({ t: 'command', ...base, command: 1 })).toBeNull();
  });

  it('rejects a command with a non-string arg', () => {
    expect(parseSyncMessage({ t: 'command', ...base, command: 'go', arg: 5 })).toBeNull();
    expect(parseSyncMessage({ t: 'command', ...base, command: 'go', arg: null })).toBeNull();
  });
});

describe('fingerprint', () => {
  it('treats undefined and null as the same no-value', () => {
    expect(fingerprint(undefined)).toBe(fingerprint(null));
    expect(fingerprint(undefined)).toBe('null');
  });

  it('distinguishes real values', () => {
    expect(fingerprint(0)).not.toBe(fingerprint(null));
    expect(fingerprint({ a: 1 })).toBe('{"a":1}');
    expect(fingerprint('a')).not.toBe(fingerprint('b'));
  });

  it('does not throw for values JSON cannot represent', () => {
    expect(fingerprint(() => 1)).toBe('null');
  });
});

describe('SyncEngine', () => {
  const msg = (origin: string, seq: number): SyncMessage => ({ t: 'bye', origin, seq, host: false });

  it('stamps messages with origin, host flag and increasing seq', () => {
    const e = new SyncEngine('me', true);
    expect(e.stamp({ t: 'bye' })).toEqual({ t: 'bye', origin: 'me', seq: 1, host: true });
    expect(e.stamp({ t: 'hello', label: 'l' })).toEqual({ t: 'hello', origin: 'me', seq: 2, host: true, label: 'l' });
    expect(e.next()).toBe(3);
  });

  it('drops its own messages', () => {
    const e = new SyncEngine('me', false);
    expect(e.accept(msg('me', 1))).toBe(false);
  });

  it('drops stale and duplicate seq per origin but tracks origins independently', () => {
    const e = new SyncEngine('me', false);
    expect(e.accept(msg('a', 5))).toBe(true);
    expect(e.accept(msg('a', 5))).toBe(false);
    expect(e.accept(msg('a', 4))).toBe(false);
    expect(e.accept(msg('a', 6))).toBe(true);
    expect(e.accept(msg('b', 1))).toBe(true);
    expect(e.accept(msg('b', 1))).toBe(false);
  });

  it('drops seq 0, which is never newer than the initial state', () => {
    expect(new SyncEngine('me', false).accept(msg('a', 0))).toBe(false);
  });

  it('shouldSend only when the value changed', () => {
    const e = new SyncEngine('me', false);
    expect(e.shouldSend('k', { a: 1 })).toBe(true);
    expect(e.shouldSend('k', { a: 1 })).toBe(false);
    expect(e.shouldSend('k', { a: 2 })).toBe(true);
    expect(e.shouldSend('other', { a: 2 })).toBe(true);
  });

  it('suppresses the echo of an applied remote value', () => {
    const e = new SyncEngine('me', false);
    e.noteApplied('k', 'remote');
    expect(e.shouldSend('k', 'remote')).toBe(false);
    expect(e.shouldSend('k', 'local')).toBe(true);
  });

  it('forget lets an origin restart its sequence', () => {
    const e = new SyncEngine('me', false);
    e.accept(msg('a', 9));
    expect(e.accept(msg('a', 1))).toBe(false);
    e.forget('a');
    expect(e.accept(msg('a', 1))).toBe(true);
    expect(() => e.forget('never-seen')).not.toThrow();
  });
});

describe('newOrigin', () => {
  it('uses the random function and is non-empty within the origin limit', () => {
    const a = newOrigin(() => 0);
    const b = newOrigin(() => 0.999);
    expect(a).toMatch(/^o[0-9a-z]+0$/);
    expect(a).not.toBe(b);
    expect(b.length).toBeLessThanOrEqual(64);
    expect(newOrigin(() => 0.5)).toContain(Math.floor(0.5 * 2 ** 32).toString(36));
  });

  it('falls back to Math.random', () => {
    expect(newOrigin()).toMatch(/^o[0-9a-z]+$/);
  });
});
