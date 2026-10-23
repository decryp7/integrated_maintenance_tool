import { StoreFolder } from './storage.types';
import { notSelectableReason, orphaned, parseAutoCleanDays, shortStoreName, stateLabel, stillOrphaned } from './stores-format';

const folder = (id: string, state: StoreFolder['state']): StoreFolder => ({ id, state, bytes: 1, files: 1, lastUsedMs: null });

describe('stores format', () => {
  it('labels every state in words', () => {
    expect(stateLabel('open')).toBe('Open');
    expect(stateLabel('session')).toBe('Kept for session restore');
    expect(stateLabel('running')).toBe('In use by another Debrief');
    expect(stateLabel('orphaned')).toBe('Orphaned');
  });

  it('explains why a store cannot be selected, except an orphaned one', () => {
    expect(notSelectableReason('open')).toContain('open');
    expect(notSelectableReason('session')).toContain('session');
    expect(notSelectableReason('running')).toContain('Another Debrief');
    expect(notSelectableReason('orphaned')).toBeNull();
  });

  it('shortens long folder names in the middle only', () => {
    expect(shortStoreName('store-1')).toBe('store-1');
    const long = 'store-0123456789abcdef0123456789abcdef';
    const short = shortStoreName(long);
    expect(short.length).toBe(20);
    expect(short).toContain('…');
    expect(short.startsWith('store-0123456')).toBe(true);
    expect(short.endsWith(long.slice(-6))).toBe(true);
  });

  it('keeps only still-orphaned ids of a selection, in order', () => {
    const folders = [folder('a', 'orphaned'), folder('b', 'open'), folder('c', 'orphaned')];
    expect(orphaned(folders).map((f) => f.id)).toEqual(['a', 'c']);
    expect(stillOrphaned(['c', 'b', 'x', 'a'], folders)).toEqual(['c', 'a']);
  });

  it('parses the days: whole numbers 0 to 365 only', () => {
    expect(parseAutoCleanDays('0')).toEqual({ days: 0 });
    expect(parseAutoCleanDays(' 30 ')).toEqual({ days: 30 });
    expect(parseAutoCleanDays('365')).toEqual({ days: 365 });
    expect(parseAutoCleanDays('366')).toEqual({ error: 'At most 365 days.' });
    expect(parseAutoCleanDays('')).toMatchObject({ error: expect.stringContaining('whole number') });
    expect(parseAutoCleanDays('-1')).toMatchObject({ error: expect.stringContaining('whole number') });
    expect(parseAutoCleanDays('1.5')).toMatchObject({ error: expect.stringContaining('whole number') });
    expect(parseAutoCleanDays('abc')).toMatchObject({ error: expect.stringContaining('whole number') });
  });
});
