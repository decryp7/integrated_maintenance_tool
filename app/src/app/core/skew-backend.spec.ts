import { TestBed } from '@angular/core/testing';
import { provideTinyBackendBare, tinyBackend } from '../../testing/helpers';
import { DebriefBackend } from './backend';
import { createSkewBackend, hasSkew, provideSkewBackend, SkewBackend, TauriSkewBackend, UnsupportedSkewBackend } from './skew-backend';

const tauriKey = '__TAURI_INTERNALS__';

describe('SkewBackend', () => {
  afterEach(() => delete (window as unknown as Record<string, unknown>)[tauriKey]);

  it('maps the methods to their Tauri commands', async () => {
    const invoke = vi.fn(async () => 'ok');
    const b = new TauriSkewBackend(invoke as never);
    await b.setSourceOffset({ kind: 'manual', source: 1, offsetMs: 5 });
    expect(invoke).toHaveBeenLastCalledWith('set_source_offset', { request: { kind: 'manual', source: 1, offsetMs: 5 } });
    await b.suggestOffsets({ reference: 2 });
    expect(invoke).toHaveBeenLastCalledWith('suggest_offsets', { request: { reference: 2 } });
    await b.suggestOffsets();
    expect(invoke).toHaveBeenLastCalledWith('suggest_offsets', { request: {} });
  });

  it('uses the mock workspace in a browser, a refusing stand-in when it cannot, and Tauri in the app', async () => {
    TestBed.configureTestingModule({ providers: [...provideTinyBackendBare(), provideSkewBackend()] });
    const skew = TestBed.inject(SkewBackend);
    expect(skew).toBe(TestBed.inject(DebriefBackend));
    expect(hasSkew(skew)).toBe(true);
    expect(hasSkew(null)).toBe(false);
    expect(hasSkew({ setSourceOffset: () => 0 })).toBe(false);

    TestBed.resetTestingModule();
    TestBed.configureTestingModule({ providers: [...provideTinyBackendBare({} as DebriefBackend)] });
    const unsupported = TestBed.runInInjectionContext(() => createSkewBackend());
    expect(unsupported).toBeInstanceOf(UnsupportedSkewBackend);
    await expect(unsupported.setSourceOffset({ kind: 'manual', source: 0, offsetMs: 0 })).rejects.toMatch(/cannot correct/);
    expect((await unsupported.suggestOffsets()).suggestions).toEqual([]);

    (window as unknown as Record<string, unknown>)[tauriKey] = {};
    TestBed.resetTestingModule();
    TestBed.configureTestingModule({ providers: [...provideTinyBackendBare(tinyBackend())] });
    const tauri = TestBed.runInInjectionContext(() => createSkewBackend());
    expect(tauri).not.toBeInstanceOf(UnsupportedSkewBackend);
    expect(typeof tauri.setSourceOffset).toBe('function');
  });
});
