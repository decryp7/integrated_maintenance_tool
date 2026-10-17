import { CounterPreset } from '../core/tools.types';
import { copyText } from './clipboard';
import { DEFAULT_PLAN, formatBytes, joinLines, newLogSpec, optionalNumber, parsePlan, splitLines, stringifyPlan, summarize } from './collector/plan-model';
import { DEFAULT_MONITOR_DRAFT, invalidRows, rowToTarget, toMonitorConfig, validateMonitor } from './monitor/monitor-model';
import { addProcess, DEFAULT_DRAFT, fullPresets, normaliseProcess, presetState, toggleCounter, togglePreset, toTemplate, validateDraft } from './perfmon/perfmon-model';
import { baseName, errorMessage, isAccessDenied } from './tool-errors';

const presets: CounterPreset[] = [
  { id: 'cpu', name: 'CPU', description: '', counters: [{ path: 'a', guidance: null }, { path: 'b', guidance: 'g' }] },
  { id: 'mem', name: 'Mem', description: '', counters: [{ path: 'c', guidance: null }] },
  { id: 'empty', name: 'Empty', description: '', counters: [] },
];

describe('tool errors', () => {
  it('formats errors and recognises missing rights', () => {
    expect(errorMessage('x')).toBe('x');
    expect(errorMessage(new Error('y'))).toBe('y');
    expect(errorMessage({ a: 1 })).toBe('{"a":1}');
    expect(isAccessDenied('Access is denied.')).toBe(true);
    expect(isAccessDenied('this requires elevation')).toBe(true);
    expect(isAccessDenied('Run as administrator')).toBe(true);
    expect(isAccessDenied('disk full')).toBe(false);
    expect(isAccessDenied(null)).toBe(false);
    expect(baseName('C:\\a\\b.txt')).toBe('b.txt');
    expect(baseName('/a/b/')).toBe('b');
    expect(baseName('')).toBe('');
  });
});

describe('clipboard', () => {
  it('reports whether copying worked', async () => {
    const write = vi.fn(async () => undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: write }, configurable: true });
    expect(await copyText('hi')).toBe(true);
    expect(write).toHaveBeenCalledWith('hi');
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: async () => Promise.reject(new Error('no')) }, configurable: true });
    expect(await copyText('hi')).toBe(false);
  });
});

describe('plan model', () => {
  it('fills defaults when parsing and keeps unknown keys for the engine to reject', () => {
    const r = parsePlan('{"name":"x","logs":[{"path":"C:\\\\L"}],"redact":{"builtIn":false},"bogus":1}');
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.plan.name).toBe('x');
      expect(r.plan.logs[0].include).toEqual(newLogSpec().include);
      expect(r.plan.redact).toEqual({ builtIn: false, ipAddresses: true, patterns: [] });
      expect(r.plan.eventLogDays).toBe(7);
      expect((r.plan as unknown as Record<string, unknown>)['bogus']).toBe(1);
    }
    expect(parsePlan('{}')).toEqual({ ok: true, plan: DEFAULT_PLAN });
  });

  it('rejects text that is not a plan object', () => {
    expect(parsePlan('{oops')).toMatchObject({ ok: false });
    expect(parsePlan('[]')).toEqual({ ok: false, error: 'The plan must be a JSON object.' });
    expect(parsePlan('null')).toMatchObject({ ok: false });
    expect(parsePlan('{oops')).toHaveProperty('error', expect.stringContaining('Not valid JSON'));
    expect(parsePlan(stringifyPlan(DEFAULT_PLAN))).toEqual({ ok: true, plan: DEFAULT_PLAN });
    expect(parsePlan('{oops')).toMatchObject({ ok: false });
    const bad = parsePlan('"x"');
    expect(bad.ok).toBe(false);
  });

  it('converts lines, numbers and sizes', () => {
    expect(splitLines(' a \r\n\n b\n')).toEqual(['a', 'b']);
    expect(joinLines(['a', 'b'])).toBe('a\nb');
    expect(optionalNumber('')).toBeNull();
    expect(optionalNumber(' 7.9 ')).toBe(7);
    expect(optionalNumber('-1')).toBeNull();
    expect(optionalNumber('x')).toBeNull();
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(2048)).toBe('2.0 KB');
    expect(formatBytes(5 * 1024 ** 3)).toBe('5.0 GB');
    expect(formatBytes(5 * 1024 ** 4)).toBe('5120.0 GB');
  });

  it('summarizes a manifest', () => {
    const s = summarize({
      name: 'n', host: 'h', platform: 'p', createdAt: 't',
      items: [{ path: 'a', kind: 'log', origin: 'o', originalBytes: 2, bytes: 1, redacted: true, redactions: 3 }],
      skipped: [{ kind: 'msinfo', what: 'msinfo32', reason: 'disabled' }],
      redaction: { email: 2, token: 2, ip: 5 },
      errors: ['boom'],
      totalBytes: 10,
      durationMs: 1,
    });
    expect(s).toEqual({
      items: 1, bytes: 10, skipped: [{ what: 'msinfo32', reason: 'disabled' }], redactionTotal: 9,
      redactionByRule: [{ rule: 'ip', count: 5 }, { rule: 'email', count: 2 }, { rule: 'token', count: 2 }], errors: ['boom'],
    });
    const none = summarize({ name: '', host: '', platform: '', createdAt: '', items: [], skipped: [], redaction: undefined as never, errors: [], totalBytes: 0, durationMs: 0 });
    expect(none.redactionTotal).toBe(0);
  });
});

describe('perfmon model', () => {
  it('normalises and deduplicates processes', () => {
    expect(normaliseProcess(' W3WP.EXE ')).toBe('W3WP');
    expect(addProcess(['w3wp'], 'W3WP.exe')).toEqual(['w3wp']);
    expect(addProcess(['a'], ' ')).toEqual(['a']);
    expect(addProcess(['a'], 'b')).toEqual(['a', 'b']);
  });

  it('tracks preset selection state and toggles', () => {
    expect(presetState(presets[0], [])).toBe('none');
    expect(presetState(presets[0], ['a'])).toBe('some');
    expect(presetState(presets[0], ['a', 'b'])).toBe('all');
    expect(togglePreset(presets[0], ['a'])).toEqual(['a', 'b']);
    expect(togglePreset(presets[0], ['a', 'b', 'c'])).toEqual(['c']);
    expect(toggleCounter(['a'], 'b')).toEqual(['a', 'b']);
    expect(toggleCounter(['a', 'b'], 'a')).toEqual(['b']);
    expect(fullPresets(presets, ['a', 'b', 'c'])).toEqual(['cpu', 'mem']);
    expect(fullPresets(presets, ['a'])).toEqual([]);
  });

  it('sends whole presets by id and the rest as extra counters', () => {
    const t = toTemplate({ ...DEFAULT_DRAFT, name: ' X ', counters: ['a', 'b', 'c2'], extraCounters: ['c2', 'z'], durationMinutes: 2 }, presets);
    expect(t).toMatchObject({ name: 'X', presets: ['cpu'], extraCounters: ['c2', 'z'], durationSecs: 120, maxSizeMb: null, format: 'csv' });
    const blg = toTemplate({ ...DEFAULT_DRAFT, format: 'blg', maxSizeMb: 100, counters: ['a'] }, presets);
    expect(blg.maxSizeMb).toBe(100);
    expect(blg.durationSecs).toBeNull();
  });

  it('finds problems early', () => {
    expect(validateDraft({ ...DEFAULT_DRAFT, counters: ['a'] })).toEqual([]);
    const all = validateDraft({ ...DEFAULT_DRAFT, name: 'a/b', intervalSecs: 0, maxSizeMb: 5, outputDir: ' ', counters: [] });
    expect(all).toHaveLength(5);
    expect(validateDraft({ ...DEFAULT_DRAFT, name: '', counters: ['a'] })).toHaveLength(1);
    expect(validateDraft({ ...DEFAULT_DRAFT, extraCounters: ['x'] })).toEqual([]);
  });
});

describe('monitor model', () => {
  it('converts rows to targets', () => {
    expect(rowToTarget({ kind: 'name', value: ' svc* ' })).toEqual({ name: 'svc*' });
    expect(rowToTarget({ kind: 'path', value: 'C:\\x\\*' })).toEqual({ path: 'C:\\x\\*' });
    expect(rowToTarget({ kind: 'pid', value: '42' })).toEqual({ pid: 42 });
    expect(rowToTarget({ kind: 'pid', value: '4x' })).toBeNull();
    expect(rowToTarget({ kind: 'pid', value: '-1' })).toBeNull();
    expect(rowToTarget({ kind: 'name', value: ' ' })).toBeNull();
  });

  it('validates and builds the config', () => {
    expect(validateMonitor(DEFAULT_MONITOR_DRAFT)).toEqual(['Add at least one process name, path or id to watch.']);
    const d = { ...DEFAULT_MONITOR_DRAFT, targets: [{ kind: 'pid' as const, value: 'x' }, { kind: 'name' as const, value: 'a' }], intervalMs: 100 };
    expect(invalidRows(d)).toEqual(['"x" is not a valid process id.']);
    expect(validateMonitor(d)).toHaveLength(2);
    expect(toMonitorConfig({ ...d, intervalMs: 500, counters: ' \\a \n\n\\b ', cpu: 'core', includeChildren: true })).toEqual({
      targets: [{ name: 'a' }], intervalMs: 500, cpu: 'core', counters: ['\\a', '\\b'], includeChildren: true,
    });
  });
});
