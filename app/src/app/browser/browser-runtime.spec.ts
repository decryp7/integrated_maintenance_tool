import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Logger } from '../core/logger';
import { MemoryLogger, settle } from '../../testing/helpers';
import { BrowserRuntimeBackend, TauriBrowserRuntimeBackend } from './browser-runtime-backend';
import { BrowserRuntimeSettings } from './browser-runtime-settings';
import { BrowserRuntimeStore } from './browser-runtime.store';
import { MockBrowserRuntimeBackend } from './mock-browser-runtime-backend';

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

function setup(search = '') {
  const backend = new MockBrowserRuntimeBackend(search);
  TestBed.configureTestingModule({ providers: [{ provide: BrowserRuntimeBackend, useValue: backend }, { provide: Logger, useClass: MemoryLogger }] });
  return backend;
}

async function render() {
  const f = TestBed.createComponent(BrowserRuntimeSettings);
  f.detectChanges();
  await settle();
  f.detectChanges();
  return f;
}
const q = (f: { nativeElement: unknown }, id: string) => (f.nativeElement as HTMLElement).querySelector<HTMLElement>(`[data-testid=${id}]`);

describe('TauriBrowserRuntimeBackend', () => {
  it('maps every method to its command', async () => {
    const calls: string[] = [];
    const b = new TauriBrowserRuntimeBackend((async (cmd: string) =>
      (calls.push(cmd), cmd === 'jobs_wait' ? { state: 'done', result: { type: 'value', value: 'installed' } } : cmd === 'browser_runtime_install' ? 3 : cmd)) as never);
    expect(await b.getRuntimeStatus()).toBe('browser_runtime_status');
    // The install is a download job: the command returns its id and the new status is the job's value.
    expect(await b.installRuntime()).toBe('installed');
    expect(await b.removeRuntime()).toBe('browser_runtime_remove');
    expect(calls).toEqual(['browser_runtime_status', 'browser_runtime_install', 'jobs_wait', 'browser_runtime_remove']);
  });
});

describe('MockBrowserRuntimeBackend', () => {
  it('starts from the query: found, none, installed, outdated; installs and removes', async () => {
    expect((await new MockBrowserRuntimeBackend('').getRuntimeStatus()).source).toBe('found');
    expect((await new MockBrowserRuntimeBackend('?mockChromium=installed').getRuntimeStatus()).installedVersion).toBe((await new MockBrowserRuntimeBackend().getRuntimeStatus()).pinnedVersion);
    const old = await new MockBrowserRuntimeBackend('?mockChromium=outdated').getRuntimeStatus();
    expect(old.installedVersion).not.toBe(old.pinnedVersion);
    const none = new MockBrowserRuntimeBackend('?mockChromium=none');
    expect(await none.getRuntimeStatus()).toMatchObject({ available: false, source: 'none', path: null });
    expect(await none.installRuntime()).toMatchObject({ available: true, source: 'installed' });
    expect(await none.removeRuntime()).toMatchObject({ source: 'found', installedVersion: null });
    await expect(new MockBrowserRuntimeBackend('?mockChromiumFail=1').installRuntime()).rejects.toThrow('download failed');
  });
});

describe('BrowserRuntimeSettings', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('shows the browser in use and its path, with nothing to download', async () => {
    setup();
    const f = await render();
    expect(q(f, 'browser-runtime-status')!.textContent).toContain('found on this computer');
    expect(q(f, 'browser-runtime-path')!.textContent).toContain('Chrome');
    expect(q(f, 'browser-runtime-remove')).toBeNull();
    expect(q(f, 'browser-runtime-update')).toBeNull();
    expect(q(f, 'browser-runtime-install')!.textContent).toContain('Download Chromium');
  });

  it('explains that research is unavailable without a browser, downloads on request and removes it again', async () => {
    const backend = setup('?mockChromium=none');
    const f = await render();
    const msg = q(f, 'browser-runtime-status')!.textContent!;
    expect(msg).toContain('No Chromium-based browser found: research and the browser modes are unavailable');
    expect(msg).toContain('pinned version 131.0.6778.204');
    const install = vi.spyOn(backend, 'installRuntime');
    (q(f, 'browser-runtime-install') as HTMLButtonElement).click();
    f.detectChanges();
    expect((q(f, 'browser-runtime-install') as HTMLButtonElement).disabled).toBe(true);
    expect(q(f, 'browser-runtime-install')!.textContent).toContain('Downloading…');
    expect(q(f, 'browser-runtime-busy')).not.toBeNull();
    await wait(100);
    f.detectChanges();
    expect(install).toHaveBeenCalledTimes(1);
    expect(q(f, 'browser-runtime-status')!.textContent).toContain('Chromium Debrief downloaded');
    expect(q(f, 'browser-runtime-install')).toBeNull();
    (q(f, 'browser-runtime-remove') as HTMLButtonElement).click();
    await settle();
    f.detectChanges();
    expect(q(f, 'browser-runtime-remove')).toBeNull();
    expect(q(f, 'browser-runtime-status')!.textContent).toContain('found on this computer');
  });

  it('offers an update when the downloaded Chromium is not the pinned one', async () => {
    setup('?mockChromium=outdated');
    const f = await render();
    expect(q(f, 'browser-runtime-outdated')!.textContent).toContain('130.0.6723.58');
    expect(q(f, 'browser-runtime-install')).toBeNull();
    (q(f, 'browser-runtime-update') as HTMLButtonElement).click();
    await wait(100);
    f.detectChanges();
    expect(q(f, 'browser-runtime-update')).toBeNull();
    expect(q(f, 'browser-runtime-remove')).not.toBeNull();
  });

  it('shows a failed download inline and retries', async () => {
    const backend = setup('?mockChromium=none&mockChromiumFail=1');
    const f = await render();
    (q(f, 'browser-runtime-install') as HTMLButtonElement).click();
    await wait(100);
    f.detectChanges();
    expect(q(f, 'browser-runtime-error')!.textContent).toContain('download failed');
    expect((q(f, 'browser-runtime-install') as HTMLButtonElement).disabled).toBe(false);
    vi.spyOn(backend, 'getRuntimeStatus').mockRejectedValueOnce('engine down');
    (q(f, 'browser-runtime-retry') as HTMLButtonElement).click();
    await settle();
    f.detectChanges();
    expect(q(f, 'browser-runtime-error')!.textContent).toContain('engine down');
    (q(f, 'browser-runtime-retry') as HTMLButtonElement).click();
    await settle();
    f.detectChanges();
    expect(q(f, 'browser-runtime-error')).toBeNull();
  });

  it('ignores a second download while one runs and a missing download for platforms without one', async () => {
    const backend = setup('?mockChromium=none');
    const store = TestBed.inject(BrowserRuntimeStore);
    const install = vi.spyOn(backend, 'installRuntime');
    const first = store.install();
    await store.install();
    await first;
    await store.remove();
    expect(install).toHaveBeenCalledTimes(1);
    vi.spyOn(backend, 'getRuntimeStatus').mockResolvedValueOnce({ available: false, source: 'none', path: null, installedVersion: null, pinnedVersion: '1', canInstall: false });
    const f = await render();
    expect(q(f, 'browser-runtime-status')!.textContent).toContain('choose one in Browser to use');
    expect(q(f, 'browser-runtime-install')).toBeNull();
  });
});
