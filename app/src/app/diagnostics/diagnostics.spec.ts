import { ErrorHandler, Injector, runInInjectionContext } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { Logger, LoggingErrorHandler } from '../core/logger';
import { MemoryLogger, provideTinyBackendBare } from '../../testing/helpers';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { CommandContext } from '../extensions/tokens';
import { DiagnosticsCommands, DIAGNOSTICS_VIEWS } from './diagnostics-commands';
import { DEFAULT_CLASSIFIER, isUnexpectedBackendError } from './error-classify';
import { ErrorSnapshotService, SNAPSHOT_CLOCK } from './error-snapshot.service';
import { mockJobs } from '../jobs/mock-jobs';
import { MockSnapshotBackend } from './mock/mock-snapshot-backend';
import { createSnapshotBackend, provideDiagnostics } from './provide-diagnostics';
import { capturedStyleProperties, encodeWithin, MAX_BYTES, ScreenCapture, scaleFor } from './screen-capture';
import { SnapshotBackend, TauriSnapshotBackend } from './snapshot-backend';
import { SnapshotErrorHandler } from './snapshot-error-handler';
import { SnapshotGate } from './snapshot-gate';
import { CapturedImage } from './snapshot.types';
import { SnapshotContext } from '../core/api.types';
import { FailureReporter } from '../core/failure-reporter';
import { at } from '../../testing/must';

const IMAGE: CapturedImage = { base64: 'iVBORw0KGgo=', mime: 'image/png', bytes: 8 };

class FakeScreen extends ScreenCapture {
  calls = 0;
  fail: Error | null = null;
  async capture() {
    this.calls++;
    if (this.fail) throw this.fail;
    return IMAGE;
  }
}

describe('SnapshotGate', () => {
  it('allows one automatic capture per interval and a fixed number per session', () => {
    const gate = new SnapshotGate({ minIntervalMs: 10_000, maxPerSession: 3 });
    expect(gate.tryAcquire(0)).toBe(true);
    expect(gate.tryAcquire(9_999)).toBe(false);
    expect(gate.tryAcquire(10_000)).toBe(true);
    expect(gate.tryAcquire(25_000)).toBe(true);
    expect(gate.tryAcquire(99_000)).toBe(false);
    expect(gate.count).toBe(3);
  });

  it('lets the user capture manually past both limits', () => {
    const gate = new SnapshotGate({ minIntervalMs: 10_000, maxPerSession: 1 });
    expect(gate.tryAcquire(0)).toBe(true);
    expect(gate.tryAcquire(1, true)).toBe(true);
    expect(gate.tryAcquire(2)).toBe(false);
  });

  it('has the documented defaults (10 s, 20 per session)', () => {
    const gate = new SnapshotGate();
    let granted = 0;
    for (let i = 0; i < 30; i++) if (gate.tryAcquire(i * 10_000)) granted++;
    expect(granted).toBe(20);
  });
});

describe('error classification', () => {
  it('skips mistakes the user can fix and keeps unexpected failures', () => {
    for (const msg of ['invalid regex: unclosed group', 'No such file or directory', 'Access is denied', 'query syntax error at 3', 'cancelled', 'profile does not compile']) {
      expect(isUnexpectedBackendError(msg), msg).toBe(false);
    }
    for (const msg of ['IMT crashed: index out of bounds', 'plugin panicked', 'channel closed', new Error('boom')]) {
      expect(isUnexpectedBackendError(msg)).toBe(true);
    }
    expect(isUnexpectedBackendError(new DOMException('x', 'AbortError'))).toBe(false);
    expect(DEFAULT_CLASSIFIER('open_paths', 'Access denied')).toBe(false);
  });
});

describe('encodeWithin / scaleFor', () => {
  const url = (mime: string, bytes: number) => `data:${mime};base64,${'A'.repeat(Math.ceil((bytes * 4) / 3))}`;

  it('keeps a PNG that fits and falls back to JPEG at quality 0.8 when it does not', () => {
    const calls: [string, number | undefined][] = [];
    const canvas = (pngBytes: number) => ({
      toDataURL: (type?: string, q?: number) => {
        calls.push([type ?? '', q]);
        return type === 'image/jpeg' ? url('image/jpeg', 1000) : url('image/png', pngBytes);
      },
    });
    expect(encodeWithin(canvas(MAX_BYTES - 10)).mime).toBe('image/png');
    expect(calls).toEqual([['image/png', undefined]]);
    calls.length = 0;
    const big = encodeWithin(canvas(MAX_BYTES + 3000));
    expect(big.mime).toBe('image/jpeg');
    expect(calls).toEqual([
      ['image/png', undefined],
      ['image/jpeg', 0.8],
    ]);
    expect(big.bytes).toBeLessThan(MAX_BYTES);
  });

  it('counts base64 padding and rejects an unusable encoding', () => {
    expect(encodeWithin({ toDataURL: () => 'data:image/png;base64,QQ==' }).bytes).toBe(1);
    expect(() => encodeWithin({ toDataURL: () => 'data:,' })).toThrow(/encode/);
  });

  it('scales down to 1600 px wide and never up', () => {
    expect(scaleFor(3200)).toBe(0.5);
    expect(scaleFor(1440)).toBe(1);
  });
});

describe('DomScreenCapture', () => {
  it('draws the body with html-to-image without fonts, skipping elements marked to ignore', async () => {
    const toCanvas = vi.fn(async (_n: unknown, _o: { filter: (n: Node) => boolean; skipFonts: boolean; pixelRatio: number }) => ({
      toDataURL: () => 'data:image/png;base64,QUJD',
    }));
    vi.doMock('html-to-image', () => ({ toCanvas }));
    vi.resetModules();
    const { DomScreenCapture } = await import('./screen-capture');
    const image = await new DomScreenCapture().capture();
    expect(image).toEqual({ mime: 'image/png', base64: 'QUJD', bytes: 3 });
    const options = at(toCanvas.mock.calls, 0)[1];
    expect(at(toCanvas.mock.calls, 0)[0]).toBe(document.body);
    expect(options.skipFonts).toBe(true);
    const toast = document.createElement('div');
    toast.setAttribute('data-snapshot-ignore', '');
    expect(options.filter(toast)).toBe(false);
    expect(options.filter(document.createElement('div'))).toBe(true);
    expect(options.filter(document.createTextNode('x'))).toBe(true);
    vi.doUnmock('html-to-image');
  });

  // Regression: Angular Material's theme puts ~165 --mat-sys-* tokens on :root. html-to-image copies every
  // property of the root's computed style onto every cloned node, which made a capture ~80% slower (several
  // seconds), so the toast came too late. Computed values already resolve var(), so the clone does not need them.
  it('copies standard and IMT properties but not Angular Material theme tokens', async () => {
    const toCanvas = vi.fn(async (_n: unknown, _o: { includeStyleProperties?: string[] }) => ({ toDataURL: () => 'data:image/png;base64,QUJD' }));
    vi.doMock('html-to-image', () => ({ toCanvas }));
    vi.resetModules();
    document.documentElement.style.setProperty('--mat-sys-primary', 'red');
    document.documentElement.style.setProperty('--accent', 'blue');
    const { DomScreenCapture } = await import('./screen-capture');
    await new DomScreenCapture().capture();
    const props = at(toCanvas.mock.calls, 0)[1].includeStyleProperties ?? [];
    expect(props.some((p) => p.startsWith('--mat-'))).toBe(false);
    document.documentElement.style.removeProperty('--mat-sys-primary');
    document.documentElement.style.removeProperty('--accent');
    vi.doUnmock('html-to-image');
  });

  it('keeps every computed property except Material tokens', () => {
    expect(capturedStyleProperties(['color', '--accent', '--mat-sys-primary', 'stroke', '--mat-slide-toggle-x'])).toEqual(['color', '--accent', 'stroke']);
  });
});

describe('ErrorSnapshotService', () => {
  let now = 0;
  let frames = 0;
  let backend: MockSnapshotBackend;
  let screen: FakeScreen;
  let logger: MemoryLogger;
  let service: ErrorSnapshotService;

  beforeEach(() => {
    now = 1_000_000;
    frames = 0;
    backend = new MockSnapshotBackend();
    screen = new FakeScreen();
    logger = new MemoryLogger();
    TestBed.configureTestingModule({
      providers: [
        { provide: SnapshotBackend, useValue: backend },
        { provide: ScreenCapture, useValue: screen },
        { provide: Logger, useValue: logger },
        { provide: SNAPSHOT_CLOCK, useValue: { now: () => now, nextFrame: async () => void frames++ } },
      ],
    });
    service = TestBed.inject(ErrorSnapshotService);
  });

  it('waits a frame, captures, saves with context and raises the toast', async () => {
    const id = await service.capture('unhandled-error', { error: new Error('Failed at /Users/me/secret/file.log') });
    expect(id).toMatch(/^2026/);
    expect(frames).toBe(1);
    const saved = at(await backend.list(), 0);
    expect(saved.reason).toBe('unhandled-error');
    expect(saved.message).toBe('Failed at <path>');
    expect(saved.stack).toContain('<path>');
    expect(saved.manual).toBe(false);
    expect(service.toast()).toEqual({ kind: 'captured', id, firstTime: true });
    expect(service.saved()).toBe(1);
    service.dismissToast();
    expect(service.toast()).toBeNull();
  });

  it('describes non-Error values and uses the active tab title as the view', async () => {
    document.body.insertAdjacentHTML('beforeend', '<div class="dv-tab dv-active-tab"> Timeline </div>');
    await service.capture('unhandled-error', { error: 'plain text' });
    await service.capture('manual');
    const list = await backend.list();
    expect(list.find((i) => i.manual)?.message).toBe('manual');
    expect(list.find((i) => !i.manual)?.message).toBe('plain text');
    expect(at(list, 0).view).toBe('Timeline');
    document.querySelector('.dv-active-tab')?.remove();
  });

  it('throttles to one automatic snapshot per 10 s and 20 per session', async () => {
    expect(await service.capture('unhandled-error')).not.toBeNull();
    now += 9_000;
    expect(await service.capture('unhandled-error')).toBeNull();
    for (let i = 0; i < 40; i++) {
      now += 10_000;
      await service.capture('unhandled-error');
    }
    expect((await backend.list()).length).toBe(20);
    expect(screen.calls).toBe(20);
    // The manual command still works.
    expect(await service.manual()).not.toBeNull();
  });

  it('never captures twice at once, so an error raised while capturing cannot recurse', async () => {
    let release!: () => void;
    screen.capture = () => new Promise((r) => (release = () => r(IMAGE)));
    const first = service.capture('unhandled-error');
    await Promise.resolve();
    service.unhandled(new Error('raised while capturing'));
    expect(await service.capture('manual')).toBeNull();
    await vi.waitFor(() => expect(release).toBeTypeOf('function'));
    release();
    expect(await first).not.toBeNull();
    expect((await backend.list()).length).toBe(1);
  });

  it('logs a failed capture and does not throw, even from the error handler path', async () => {
    screen.fail = new Error('canvas tainted at /tmp/x');
    await expect(service.capture('unhandled-error')).resolves.toBeNull();
    expect(logger.entries.at(-1)).toMatchObject({ level: 'warn', category: 'ui.snapshot', message: 'Could not capture an error snapshot' });
    expect(logger.entries.at(-1)?.props?.['Error']).toBe('canvas tainted at <path>');
    expect(service.toast()).toBeNull();
    // A failed manual capture tells the user.
    now += 20_000;
    await service.manual();
    expect(service.toast()).toEqual({ kind: 'failed', id: null, firstTime: false });
    // A failing backend is handled the same way, and the service keeps working afterwards.
    screen.fail = null;
    backend.save = async () => {
      throw new Error('disk full');
    };
    now += 20_000;
    await expect(service.capture('panel-failed')).resolves.toBeNull();
  });

  it('classifies backend failures: unexpected ones only by default, the caller may decide', async () => {
    service.backendFailed('query_page', 'invalid regex');
    await vi.waitFor(() => expect(screen.calls).toBe(0));
    service.backendFailed('query_page', 'IMT crashed: bad index');
    await vi.waitFor(async () => expect((await backend.list()).length).toBe(1));
    expect((await backend.list())[0]).toMatchObject({ reason: 'backend-failed', message: 'Command query_page failed: IMT crashed: bad index' });
    now += 20_000;
    service.backendFailed('open_paths', 'invalid path', () => true);
    await vi.waitFor(async () => expect((await backend.list()).length).toBe(2));
    now += 20_000;
    service.backendFailed('x', 'whatever', () => false);
    expect(screen.calls).toBe(2);
  });

  it('captures panel failures and engine events', async () => {
    service.panelFailed('Timeline', new Error('render broke'));
    await vi.waitFor(async () => expect((await backend.list()).length).toBe(1));
    now += 20_000;
    await service.start();
    await service.start();
    backend.emit({ kind: 'plugin-quarantined', message: 'too many crashes', plugin: 'net.x.web' });
    await vi.waitFor(async () => expect((await backend.list()).length).toBe(2));
    const list = await backend.list();
    expect(list[0]).toMatchObject({ reason: 'engine-error', message: 'plugin-quarantined (net.x.web): too many crashes' });
    expect(list[1]).toMatchObject({ reason: 'panel-failed', message: 'Panel "Timeline" failed: render broke' });
    now += 20_000;
    backend.emit({ kind: 'import-crash', message: 'bad file', plugin: null });
    await vi.waitFor(async () => expect(at((await backend.list()), 0).message).toBe('import-crash: bad file'));
    service.stop();
    now += 20_000;
    backend.emit({ kind: 'import-crash', message: 'ignored', plugin: null });
    expect(at((await backend.list()), 0).message).toBe('import-crash: bad file');
  });

  it('does nothing automatic while the setting is off, but the manual command works', async () => {
    backend.enabled = false;
    await service.start();
    expect(await service.capture('unhandled-error')).toBeNull();
    expect(screen.calls).toBe(0);
    expect(await service.manual()).not.toBeNull();
    backend.enabled = true;
    await service.refreshEnabled();
    now += 20_000;
    expect(await service.capture('unhandled-error')).not.toBeNull();
    backend.automaticEnabled = async () => {
      throw new Error('gone');
    };
    await service.refreshEnabled();
    now += 20_000;
    expect(await service.capture('unhandled-error')).not.toBeNull();
  });

  it('survives a backend that cannot start', async () => {
    backend.onEngineError = async () => {
      throw new Error('no events');
    };
    await service.start();
    expect(logger.entries.at(-1)?.message).toBe('Could not start error snapshots');
  });
});

describe('SnapshotErrorHandler', () => {
  it('logs, then asks for a snapshot, and never throws', () => {
    const unhandled = vi.fn();
    const handled: unknown[] = [];
    TestBed.configureTestingModule({
      providers: [
        { provide: LoggingErrorHandler, useValue: { handleError: (e: unknown) => handled.push(e) } },
        { provide: ErrorSnapshotService, useValue: { unhandled } },
        SnapshotErrorHandler,
      ],
    });
    const handler = TestBed.inject(SnapshotErrorHandler);
    const error = new Error('x');
    handler.handleError(error);
    expect(handled).toEqual([error]);
    expect(unhandled).toHaveBeenCalledWith(error);

    unhandled.mockImplementation(() => {
      throw new Error('snapshot service broke');
    });
    expect(() => handler.handleError(error)).not.toThrow();
    expect(handled.length).toBe(2);
  });

  it('is what provideDiagnostics registers for ErrorHandler', () => {
    TestBed.configureTestingModule({ providers: [...provideTinyBackendBare(), provideDiagnostics()] });
    expect(TestBed.inject(ErrorHandler)).toBeInstanceOf(SnapshotErrorHandler);
    expect(TestBed.inject(SnapshotBackend)).toBeTruthy();
    expect(TestBed.inject(FailureReporter)).toBe(TestBed.inject(ErrorSnapshotService));
    document.querySelectorAll('imt-snapshot-toast').forEach((e) => e.remove());
  });
});

describe('commands and views', () => {
  it('registers the capture command on Mod+Shift+F12 and the view', async () => {
    const manual = vi.fn(async () => 'id');
    const showView = vi.fn();
    TestBed.configureTestingModule({ providers: [{ provide: ErrorSnapshotService, useValue: { manual } }] });
    const { commands } = TestBed.inject(DiagnosticsCommands);
    const capture = commands.find((c) => c.id === 'diagnostics.capture')!;
    expect(capture.keybinding).toBe('Mod+Shift+F12');
    expect(capture.title).toBe('Capture diagnostic screenshot');
    await capture.run({ showView } as unknown as CommandContext);
    expect(manual).toHaveBeenCalled();
    commands.find((c) => c.id === 'diagnostics.snapshots.show')!.run({ showView } as unknown as CommandContext);
    expect(showView).toHaveBeenCalledWith('error-snapshots');
    expect(at(DIAGNOSTICS_VIEWS, 0).id).toBe('error-snapshots');
    expect((await at(DIAGNOSTICS_VIEWS, 0).component()).name).toContain('ErrorSnapshotsView');
  });
});

describe('backends', () => {
  it('the Tauri backend maps each method to its command', async () => {
    const invoke = vi.fn(async (cmd: string, _a?: unknown): Promise<unknown> => (cmd === 'diagnostics_settings' ? { snapshots: { enabled: false } } : 'ok'));
    const listen = vi.fn(async (_e: string, h: (p: unknown) => void) => {
      h({ kind: 'import-crash', message: 'm', plugin: null });
      return () => undefined;
    });
    const b = new TauriSnapshotBackend(invoke as never, listen);
    const ctx: SnapshotContext = { reason: 'manual', message: 'm', manual: true };
    await b.save(IMAGE, ctx);
    await b.list();
    await b.image('a');
    await b.remove('a');
    await b.openFolder();
    await b.exportDiagnostics('/x.zip', ['a']);
    expect(await b.automaticEnabled()).toBe(false);
    expect(invoke.mock.calls).toEqual([
      ['save_error_snapshot', { image: IMAGE.base64, mime: 'image/png', context: ctx }],
      ['list_error_snapshots'],
      ['read_error_snapshot', { id: 'a' }],
      ['delete_error_snapshot', { id: 'a' }],
      ['open_snapshots_folder'],
      ['export_diagnostics', { path: '/x.zip', ids: ['a'] }],
      ['diagnostics_settings'],
    ]);
    const seen: unknown[] = [];
    await b.onEngineError((e) => seen.push(e));
    expect(at(listen.mock.calls, 0)[0]).toBe('imt://error');
    expect(seen).toEqual([{ kind: 'import-crash', message: 'm', plugin: null }]);
  });

  it('the mock backend stores, lists newest first, serves images and removes', async () => {
    const m = new MockSnapshotBackend();
    const a = await m.save(IMAGE, { reason: 'unhandled-error', message: 'one', manual: false });
    const b = await m.save(IMAGE, { reason: 'unhandled-error', message: 'two', manual: true });
    expect(a.firstTime).toBe(true);
    expect(b.firstTime).toBe(false);
    expect((await m.list()).map((i) => i.message)).toEqual(['two', 'one']);
    expect(await m.image(a.id)).toBe('data:image/png;base64,iVBORw0KGgo=');
    await m.remove(a.id);
    await expect(m.image(a.id)).rejects.toThrow();
    await expect(m.remove(a.id)).rejects.toThrow();
    expect(await m.openFolder()).toContain('snapshots');
    // The zip is written by an export job that is returned at once; Show in folder reveals the file.
    const job = await m.exportDiagnostics('/o.zip', [b.id]);
    expect(job).toEqual(expect.any(Number));
    const end = await mockJobs().wait(job);
    expect([end.state, end.message, end.result]).toEqual(['done', '3 files zipped.', { type: 'file', path: '/o.zip' }]);
    m.enabled = false;
    await expect(m.save(IMAGE, { reason: 'unhandled-error', message: 'm', manual: false })).rejects.toThrow(/turned off/);
  });

  it('chooses the mock in a browser', async () => {
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useValue: new MemoryLogger() }] });
    const backend = runInInjectionContext(TestBed.inject(Injector), () => createSnapshotBackend());
    return expect(backend.automaticEnabled()).resolves.toBe(true);
  });

  it('unused context token stays injectable', () => {
    expect(COMMAND_CONTEXT).toBeTruthy();
  });
});
