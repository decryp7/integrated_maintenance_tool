import { TestBed } from '@angular/core/testing';
import { MemoryLogger } from '../../testing/helpers';
import { Logger } from '../core/logger';
import { MockSettingsBackend, mockSections, mockValidate } from './mock-settings-backend';
import { SettingsBackend, TauriSettingsBackend } from './settings-backend';
import { FieldKind, SettingField } from './settings.types';

describe('TauriSettingsBackend', () => {
  it('maps every method to its command', async () => {
    const calls: [string, unknown][] = [];
    const call = vi.fn(async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return null;
    });
    const b = new TauriSettingsBackend(call as never);
    await b.sections();
    await b.set('workspace', { liveTailDebounce: 900 });
    await b.reset('workspace');
    await b.exportTo('/x.json');
    await b.importFrom('/y.json');
    expect(calls).toEqual([
      ['settings_sections', undefined],
      ['set_settings', { section: 'workspace', values: { liveTailDebounce: 900 } }],
      ['reset_settings', { section: 'workspace' }],
      ['export_settings', { path: '/x.json' }],
      ['import_settings', { path: '/y.json' }],
    ]);
  });

  it('is the mock in a browser, with failures logged', async () => {
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useFactory: () => new MemoryLogger() }] });
    const backend = TestBed.inject(SettingsBackend);
    expect((await backend.sections()).sections.length).toBeGreaterThan(3);
    await expect(backend.set('nope', {})).rejects.toContain('no settings section');
    expect((TestBed.inject(Logger) as MemoryLogger).entries[0]?.props?.['Command']).toBe('set');
  });
});

describe('MockSettingsBackend', () => {
  const f = (kind: FieldKind): SettingField => ({ key: 'k', label: 'K', description: '', kind, default: 0, restartRequired: false, advanced: false });

  it('mimics the engine validation messages', () => {
    expect(mockValidate(f({ type: 'bool' }), 1)).toContain('true or false');
    expect(mockValidate(f({ type: 'bool' }), true)).toBeNull();
    const int = f({ type: 'int', min: 1, max: 9, step: 2, unit: 'MB' });
    expect(mockValidate(int, 'x')).toBe('Must be a whole number.');
    expect(mockValidate(int, 11)).toBe('Must be between 1 MB and 9 MB.');
    expect(mockValidate(int, 4)).toBe('Must be 1 plus a multiple of 2.');
    expect(mockValidate(int, 3)).toBeNull();
    const fl = f({ type: 'float', min: 0, max: 1, step: 0.1, unit: null });
    expect(mockValidate(fl, 'x')).toBe('Must be a number.');
    expect(mockValidate(fl, 2)).toBe('Must be between 0 and 1.');
    expect(mockValidate(fl, 0.5)).toBeNull();
    const text = f({ type: 'text', maxLen: 3, pattern: '[a-z]+' });
    expect(mockValidate(text, 1)).toBe('Must be text.');
    expect(mockValidate(text, 'abcd')).toBe('Must be at most 3 characters.');
    expect(mockValidate(text, 'AB')).toContain('expected format');
    expect(mockValidate(f({ type: 'text', maxLen: 3, pattern: null }), 'AB')).toBeNull();
    expect(mockValidate(f({ type: 'enum', options: [{ value: 'a', label: 'A' }] }), 'b')).toBe('Must be one of: a.');
    expect(mockValidate(f({ type: 'enum', options: [{ value: 'a', label: 'A' }] }), 'a')).toBeNull();
    const path = f({ type: 'path', dir: true });
    expect(mockValidate(path, 1)).toBe('Must be a path.');
    expect(mockValidate(path, 'rel')).toContain('absolute');
    expect(mockValidate(path, 'C:\\x')).toBeNull();
    expect(mockValidate(path, '')).toBeNull();
    const dur = f({ type: 'duration', minMs: 1000, maxMs: 60_000 });
    expect(mockValidate(dur, -1)).toContain('whole milliseconds');
    expect(mockValidate(dur, 10)).toBe('Must be between 1 s and 1 min.');
    expect(mockValidate(dur, 1000)).toBeNull();
    const size = f({ type: 'size', minBytes: 1024, maxBytes: 2048 });
    expect(mockValidate(size, 1.5)).toContain('whole bytes');
    expect(mockValidate(size, 10)).toBe('Must be between 1 KB and 2 KB.');
    expect(mockValidate(size, 1024)).toBeNull();
    const list = f({ type: 'stringList', maxItems: 1 });
    expect(mockValidate(list, 'a')).toBe('Must be a list of text.');
    expect(mockValidate(list, ['a', 'b'])).toBe('Must have at most 1 items.');
    expect(mockValidate(list, [1])).toBe('Items must be text.');
    expect(mockValidate(list, ['a'])).toBeNull();
  });

  it('applies, rejects, resets, exports and imports like the engine', async () => {
    const b = new MockSettingsBackend();
    const bad = await b.set('workspace', { liveTailDebounce: 5 });
    expect(bad).toEqual({ applied: false, restartRequired: false, errors: [{ key: 'liveTailDebounce', message: 'Must be between 100 ms and 10 s.' }], values: { liveTailDebounce: 750 } });
    const ok = await b.set('diagnostics', { logMaxFileMb: 20, logLevel: null });
    expect(ok.applied && ok.restartRequired).toBe(true);
    expect(ok.values['logLevel']).toBe('info');
    expect((await b.sections()).restartPending).toBe(true);
    await expect(b.set('com.example.demo', {})).rejects.toContain('disabled');
    await expect(b.set('ai', {})).rejects.toContain('not installed');
    expect((await b.reset('diagnostics')).values['logMaxFileMb']).toBe(10);
    await b.exportTo('/e.json');
    b.files.set('/i.json', JSON.stringify({ version: 1, sections: { workspace: { liveTailDebounce: 400 }, diagnostics: { logKeepFiles: 0 }, 'com.example.demo': {}, 'x.y': {} } }));
    const r = await b.importFrom('/i.json');
    expect(r).toEqual({ applied: ['workspace'], kept: ['com.example.demo', 'x.y'], rejected: [{ section: 'diagnostics', errors: [{ key: 'logKeepFiles', message: 'Must be between 1 and 50.' }] }], restartRequired: false });
    expect((await b.importFrom('/e.json')).applied).toContain('diagnostics');
    await expect(b.importFrom('/missing.json')).rejects.toContain('no such file');
    const sections = mockSections();
    expect(sections.find((s) => s.id === 'plugins')?.values['com.example.demo']).toBe(false);
  });

  it('can fail every call', async () => {
    const b = new MockSettingsBackend({ fail: true });
    await expect(b.sections()).rejects.toContain('unavailable');
    expect(b.calls).toEqual(['sections']);
  });

  it('reports restart only when a restart field changed', async () => {
    const b = new MockSettingsBackend();
    const r = await b.set('diagnostics', { logLevel: 'debug' });
    expect(r.restartRequired).toBe(false);
    const imp = new MockSettingsBackend();
    imp.files.set('/r.json', JSON.stringify({ sections: { plugins: { 'net.decryptology.web': false } } }));
    expect((await imp.importFrom('/r.json')).restartRequired).toBe(true);
  });
});
