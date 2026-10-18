import { mockSections } from './mock-settings-backend';
import { displayValue, editUnit, fieldMatches, formatBytes, formatMs, parseList, parseNumber, precheck, rangeHint, sameValue, statusLabel } from './settings-model';
import { FieldKind, SettingField, SettingsSection } from './settings.types';
import { at } from '../../testing/must';

const field = (kind: FieldKind, def: SettingField['default'] = 0, extra: Partial<SettingField> = {}): SettingField => ({
  key: 'k',
  label: 'Label',
  description: 'Some help',
  kind,
  default: def,
  restartRequired: false,
  advanced: false,
  ...extra,
});

describe('settings model', () => {
  it('formats durations and sizes in the largest whole unit', () => {
    expect([0, 1500, 90_000, 120_000, 7_200_000].map(formatMs)).toEqual(['0 ms', '1500 ms', '90 s', '2 min', '2 h']);
    expect([0, 1000, 2048, 10 << 20, 3 * (1 << 30)].map(formatBytes)).toEqual(['0 bytes', '1000 bytes', '2 KB', '10 MB', '3 GB']);
  });

  it('picks the unit a duration or size is edited in', () => {
    expect(editUnit({ type: 'duration', minMs: 5000, maxMs: 3_600_000 })).toEqual({ factor: 1000, label: 's' });
    expect(editUnit({ type: 'duration', minMs: 100, maxMs: 10_000 })).toEqual({ factor: 1, label: 'ms' });
    expect(editUnit({ type: 'size', minBytes: 1 << 20, maxBytes: 1 << 30 })).toEqual({ factor: 1 << 20, label: 'MB' });
    expect(editUnit({ type: 'size', minBytes: 0, maxBytes: 100 })).toEqual({ factor: 1, label: 'bytes' });
    expect(editUnit({ type: 'bool' })).toBeNull();
  });

  it('describes limits', () => {
    expect(rangeHint({ type: 'int', min: 1, max: 100, step: 1, unit: 'MB' })).toBe('1–100 MB');
    expect(rangeHint({ type: 'float', min: 0, max: 1, step: 0.1, unit: null })).toBe('0–1');
    expect(rangeHint({ type: 'duration', minMs: 5000, maxMs: 3_600_000 })).toBe('5 s – 1 h');
    expect(rangeHint({ type: 'size', minBytes: 1024, maxBytes: 1 << 20 })).toBe('1 KB – 1 MB');
    expect(rangeHint({ type: 'text', maxLen: 5, pattern: null })).toBe('up to 5 characters');
    expect(rangeHint({ type: 'stringList', maxItems: 3 })).toBe('up to 3 items, one per line');
    expect(rangeHint({ type: 'path', dir: true })).toBe('folder');
    expect(rangeHint({ type: 'path', dir: false })).toBe('file');
    expect(rangeHint({ type: 'bool' })).toBe('');
  });

  it('displays values the way users read them', () => {
    expect(displayValue(field({ type: 'bool' }), true)).toBe('on');
    expect(displayValue(field({ type: 'bool' }), false)).toBe('off');
    expect(displayValue(field({ type: 'duration', minMs: 0, maxMs: 1 }), 120_000)).toBe('2 min');
    expect(displayValue(field({ type: 'size', minBytes: 0, maxBytes: 1 }), 2048)).toBe('2 KB');
    const e = field({ type: 'enum', options: [{ value: 'a', label: 'Alpha' }] });
    expect(displayValue(e, 'a')).toBe('Alpha');
    expect(displayValue(e, 'b')).toBe('b');
    expect(displayValue(field({ type: 'stringList', maxItems: 3 }), [])).toBe('none');
    expect(displayValue(field({ type: 'stringList', maxItems: 3 }), ['x', 'y'])).toBe('x, y');
    expect(displayValue(field({ type: 'int', min: 0, max: 9, step: 1, unit: 'days' }), 3)).toBe('3 days');
    expect(displayValue(field({ type: 'int', min: 0, max: 9, step: 1, unit: null }), 3)).toBe('3');
    expect(displayValue(field({ type: 'path', dir: true }), '')).toBe('not set');
  });

  it('compares values structurally', () => {
    expect(sameValue(['a'], ['a'])).toBe(true);
    expect(sameValue(1, '1')).toBe(false);
  });

  it('matches search words against label, description, key and section title', () => {
    const section = { title: 'Diagnostics' } as SettingsSection;
    const f = field({ type: 'bool' }, true, { key: 'logLevel', label: 'Log level', description: 'How much is written' });
    expect(fieldMatches(section, f, 'log')).toBe(true);
    expect(fieldMatches(section, f, 'LOGLEVEL')).toBe(true);
    expect(fieldMatches(section, f, 'written much')).toBe(true);
    expect(fieldMatches(section, f, 'diagnostics level')).toBe(true);
    expect(fieldMatches(section, f, 'timeout')).toBe(false);
  });

  it('pre-checks ranges for convenience', () => {
    const n = field({ type: 'int', min: 1, max: 10, step: 1, unit: 'MB' });
    expect(precheck(n, 5)).toBeNull();
    expect(precheck(n, 50)).toBe('Between 1–10 MB.');
    expect(precheck(n, 'x')).toBe('Enter a number.');
    const d = field({ type: 'duration', minMs: 1000, maxMs: 2000 });
    expect(precheck(d, 1500)).toBeNull();
    expect(precheck(d, 3000)).toBe('Between 1 s – 2 s.');
    expect(precheck(d, '')).toBe('Enter a number.');
    const s = field({ type: 'size', minBytes: 1024, maxBytes: 2048 });
    expect(precheck(s, 10)).toBe('Between 1 KB – 2 KB.');
    expect(precheck(field({ type: 'text', maxLen: 2, pattern: null }), 'abc')).toBe('At most 2 characters.');
    expect(precheck(field({ type: 'text', maxLen: 2, pattern: null }), 'ab')).toBeNull();
    expect(precheck(field({ type: 'stringList', maxItems: 1 }), ['a', 'b'])).toBe('At most 1 items.');
    expect(precheck(field({ type: 'stringList', maxItems: 1 }), ['a'])).toBeNull();
    expect(precheck(field({ type: 'bool' }), 'x')).toBeNull();
  });

  it('parses numbers and lists from inputs', () => {
    expect(parseNumber('12')).toBe(12);
    expect(parseNumber(' 1.5 ', 1000)).toBe(1500);
    expect(parseNumber('')).toBe('');
    expect(parseNumber('abc')).toBe('abc');
    expect(parseList('a\n\n b \r\nc')).toEqual(['a', 'b', 'c']);
  });

  it('labels section statuses', () => {
    const sections = mockSections();
    const first = at(sections, 0);
    const as = (status: SettingsSection['status']) => statusLabel({ ...first, status });
    expect([as('active'), as('disabled'), as('quarantined'), as('notInstalled'), as('safeMode')]).toEqual([null, 'disabled', 'stopped', 'not installed', 'safe mode']);
  });
});
