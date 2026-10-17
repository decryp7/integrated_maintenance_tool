import { TestBed } from '@angular/core/testing';
import { buttonByText, provideTinyBackend, settle, text } from '../../testing/helpers';
import { ImportProgress, WorkspaceSummary } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { WorkspaceStore } from '../core/workspace.store';
import { UxBackend } from '../core/ux-backend';
import { fileName, formatBytes, formatEta, percent, progressLine } from './import-format';
import { ImportProgressService, isImportLabel, LONG_IMPORT_MS, POLL_MS } from './import-progress.service';
import { ImportStatus, ImportToast, PartialImportBanner } from './import-ui';

const p = (patch: Partial<ImportProgress> = {}): ImportProgress => ({
  running: true,
  done: 3,
  total: 12,
  cancelled: false,
  bytesDone: 24_000_000,
  bytesTotal: 80_000_000,
  current: '/logs/web01/app.log',
  elapsedMs: 3000,
  etaMs: 12_000,
  ...patch,
});

describe('import formatting', () => {
  it('formats bytes, ETA, file names, percent and the line', () => {
    expect(formatBytes(999)).toBe('999 B');
    expect(formatBytes(1500)).toBe('1.5 KB');
    expect(formatBytes(24_000_000)).toBe('24 MB');
    expect(formatBytes(5e15)).toBe('5000 TB');
    expect(formatEta(null)).toBe('');
    expect(formatEta(-1)).toBe('');
    expect(formatEta(Infinity)).toBe('');
    expect(formatEta(500)).toBe('almost done');
    expect(formatEta(12_000)).toBe('about 12 s left');
    expect(formatEta(150_000)).toBe('about 3 min left');
    expect(formatEta(7_200_000)).toBe('about 2.0 h left');
    expect(fileName(null)).toBe('');
    expect(fileName('C:\\logs\\a.log')).toBe('a.log');
    expect(fileName('/')).toBe('/');
    expect(percent(p())).toBe(30);
    expect(percent(p({ bytesTotal: 0 }))).toBe(25);
    expect(percent(p({ bytesTotal: 0, total: 0 }))).toBe(0);
    expect(progressLine(p())).toBe('3 of 12 files · 24 MB of 80 MB · about 12 s left');
    expect(progressLine(p({ total: 1, done: 0, bytesTotal: 0, etaMs: null }))).toBe('0 of 1 file');
  });

  it('recognises import work by its busy label', () => {
    expect(isImportLabel('Opening 2 items…')).toBe(true);
    expect(isImportLabel('Re-importing…')).toBe(true);
    expect(isImportLabel('Removing source…')).toBe(false);
    expect(isImportLabel(null)).toBe(false);
  });
});

/** A UxBackend whose progress the test sets. */
class FakeUx {
  progress: ImportProgress = p();
  fail = false;
  cancelFail: unknown = null;
  cancelled = 0;
  importProgress = vi.fn(async () => {
    if (this.fail) throw new Error('no');
    return this.progress;
  });
  cancelImport = vi.fn(async () => {
    if (this.cancelFail) throw this.cancelFail;
    this.cancelled++;
  });
}

describe('ImportProgressService and its widgets', () => {
  let ux: FakeUx;
  let release: (s: WorkspaceSummary) => void;

  function setup() {
    ux = new FakeUx();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), { provide: UxBackend, useValue: ux }] });
    const backend = TestBed.inject(ImtBackend);
    vi.spyOn(backend, 'openPaths').mockImplementation(() => new Promise<WorkspaceSummary>((r) => (release = r)));
    return { svc: TestBed.inject(ImportProgressService), store: TestBed.inject(WorkspaceStore), backend };
  }

  afterEach(() => vi.useRealTimers());

  it('polls while opening, shows the status and the toast for long imports, and cancels', async () => {
    const { svc, store } = setup();
    await settle();
    const summary = store.summary()!;
    vi.useFakeTimers();
    const status = TestBed.createComponent(ImportStatus);
    const toast = TestBed.createComponent(ImportToast);
    const done = store.openPaths(['/big']);
    TestBed.tick();
    expect(svc.active()).toBe(true);
    await vi.advanceTimersByTimeAsync(POLL_MS);
    status.detectChanges();
    expect(text(status)).toContain('Importing 3 of 12 files · 24 MB of 80 MB · about 12 s left');
    expect(text(status)).toContain('app.log');
    const bar = status.nativeElement.querySelector('[role=progressbar]') as HTMLElement;
    expect(bar.getAttribute('aria-valuenow')).toBe('30');
    toast.detectChanges();
    expect(text(toast)).toBe('');
    await vi.advanceTimersByTimeAsync(LONG_IMPORT_MS);
    toast.detectChanges();
    expect(text(toast)).toContain('Reading app.log');
    buttonByText(toast, 'Cancel import').click();
    await vi.advanceTimersByTimeAsync(0);
    status.detectChanges();
    expect(ux.cancelled).toBe(1);
    expect(text(status)).toContain('Cancelling…');
    await svc.cancel();
    expect(ux.cancelled).toBe(1);
    // Finished with only some files read: the banner says so.
    ux.progress = p({ running: false, done: 5, total: 12, cancelled: true, current: null });
    release(summary);
    await done;
    TestBed.tick();
    await vi.advanceTimersByTimeAsync(0);
    expect(svc.active()).toBe(false);
    expect(svc.partial()).toEqual({ done: 5, total: 12 });
    vi.useRealTimers();
    const banner = TestBed.createComponent(PartialImportBanner);
    banner.detectChanges();
    expect(text(banner)).toContain('Partially imported: 5 of 12 files were read');
    buttonByText(banner, 'Dismiss').click();
    expect(svc.partial()).toBeNull();
  });

  it('re-imports all from the banner and shows no banner after a complete import', async () => {
    const { svc, store, backend } = setup();
    await settle();
    ux.progress = p({ running: false, done: 12, total: 12 });
    const done = store.openPaths(['/a']);
    await settle();
    release(store.summary()!);
    await done;
    await settle();
    expect(svc.partial()).toBeNull();
    svc.partial.set({ done: 1, total: 2 });
    const reload = vi.spyOn(backend, 'reloadWorkspace');
    const banner = TestBed.createComponent(PartialImportBanner);
    banner.detectChanges();
    buttonByText(banner, 'Re-import all').click();
    await settle();
    expect(reload).toHaveBeenCalled();
    expect(svc.partial()).toBeNull();
  });

  it('keeps going when progress cannot be read, reports a failed cancel and ignores cancel when idle', async () => {
    const { svc, store } = setup();
    await settle();
    await svc.cancel();
    expect(ux.cancelImport).not.toHaveBeenCalled();
    ux.fail = true;
    const done = store.openPaths(['/a']);
    await settle();
    expect(svc.active()).toBe(true);
    ux.cancelFail = new Error('denied');
    await svc.cancel();
    expect(store.error()).toBe('Could not cancel the import: denied');
    expect(svc.cancelling()).toBe(false);
    ux.cancelFail = 'plain';
    await svc.cancel();
    expect(store.error()).toBe('Could not cancel the import: plain');
    release(store.summary()!);
    await done;
    await settle();
    expect(svc.active()).toBe(false);
    expect(svc.partial()).toBeNull();
  });

  it('ignores a progress reply that arrives after the import ended', async () => {
    const { svc, store } = setup();
    await settle();
    ux.progress = p({ running: false });
    const done = store.openPaths(['/a']);
    await settle();
    expect(svc.progress()?.total).toBe(0);
    release(store.summary()!);
    await done;
    TestBed.resetTestingModule();
  });
});
