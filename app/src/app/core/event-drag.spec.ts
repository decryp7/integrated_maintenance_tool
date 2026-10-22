import { describe, expect, it } from 'vitest';
import { EVENT_DRAG_TYPE, hasEventDrag, readEventDrag, writeEventDrag } from './event-drag';

/** A minimal DataTransfer: jsdom has none. */
function transfer(initial: Record<string, string> = {}): DataTransfer {
  const data = new Map(Object.entries(initial));
  return {
    effectAllowed: 'none',
    get types() {
      return [...data.keys()];
    },
    setData: (type: string, value: string) => void data.set(type, value),
    getData: (type: string) => data.get(type) ?? '',
  } as unknown as DataTransfer;
}

const record = { id: 7, ts: 1_700_000_000_000, sourceName: 'app.log', message: 'first line\nsecond line' };

describe('event drag', () => {
  it('carries the event as JSON and its message as text', () => {
    const dt = transfer();
    writeEventDrag(dt, record);
    expect(dt.effectAllowed).toBe('copy');
    expect(dt.getData('text/plain')).toBe('first line\nsecond line');
    expect(hasEventDrag(dt)).toBe(true);
    expect(readEventDrag(dt)).toEqual({ id: 7, ts: 1_700_000_000_000, source: 'app.log', excerpt: 'first line' });
  });

  it('cuts a long first line', () => {
    const dt = transfer();
    writeEventDrag(dt, { ...record, message: 'x'.repeat(1000) });
    expect(readEventDrag(dt)?.excerpt).toHaveLength(300);
  });

  it('knows nothing is dragged without the type, and without a transfer', () => {
    expect(hasEventDrag(transfer({ 'text/plain': 'hi' }))).toBe(false);
    expect(hasEventDrag(null)).toBe(false);
    expect(readEventDrag(null)).toBeNull();
    expect(readEventDrag(transfer())).toBeNull();
  });

  it.each([
    ['not JSON', 'nope'],
    ['not an object', '42'],
    ['null', 'null'],
    ['no id', '{"ts":1}'],
    ['a string id', '{"id":"7"}'],
    ['a negative id', '{"id":-1}'],
    ['a fractional id', '{"id":1.5}'],
    ['an unsafe id', '{"id":1e300}'],
  ])('ignores a drop that carries %s', (_name, payload) => {
    expect(readEventDrag(transfer({ [EVENT_DRAG_TYPE]: payload }))).toBeNull();
  });

  it('defaults the optional parts of a sparse or odd payload', () => {
    expect(readEventDrag(transfer({ [EVENT_DRAG_TYPE]: '{"id":3,"ts":"x","source":5,"excerpt":null}' }))).toEqual({ id: 3, ts: null, source: '', excerpt: '' });
    expect(readEventDrag(transfer({ [EVENT_DRAG_TYPE]: JSON.stringify({ id: 3, ts: null, source: 's'.repeat(5000), excerpt: 'e'.repeat(5000) }) }))).toMatchObject({ source: 's'.repeat(1000), excerpt: 'e'.repeat(300) });
  });
});
