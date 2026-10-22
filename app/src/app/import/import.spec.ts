import { TestBed } from '@angular/core/testing';
import { buttonByText, provideTinyBackend, settle, text } from '../../testing/helpers';
import { ImportProgress, WorkspaceSummary } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { WorkspaceStore } from '../core/workspace.store';
import { UxBackend } from '../core/ux-backend';
import { fileName, importStatusText, percent, progressLine } from './import-format';
import { WelcomeService } from '../welcome/welcome';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { ImportProgressService, jobImportProgress, LONG_IMPORT_MS } from './import-progress.service';
import { JobsStore } from '../jobs/jobs.store';
import { JobInfo } from '../jobs/jobs.types';
import { WorkspaceStatusStore } from '../core/workspace/status.store';
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
    expect(fileName(null)).toBe('');
    expect(fileName('C:\\logs\\a.log')).toBe('a.log');
    expect(fileName('/')).toBe('/');
    expect(percent(p())).toBe(30);
    expect(percent(p({ bytesTotal: 0 }))).toBe(25);
    expect(percent(p({ bytesTotal: 0, total: 0 }))).toBe(0);
    expect(progressLine(p())).toBe('3 of 12 files · 23 MB of 76 MB · less than a minute left');
    expect(progressLine(p({ total: 1, done: 0, bytesTotal: 0, etaMs: null }))).toBe('0 of 1 file');
    // Before the engine counted the files there is no "0 of 0 files" (E1).
    expect(importStatusText(p({ total: 0, done: 0 }))).toBe('Scanning for files…');
    expect(importStatusText(p({ bytesTotal: 0, etaMs: null }))).toBe('Importing 3 of 12 files');
  });

  it('recognises import work by the kind of the busy state, not its label', () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const status = TestBed.inject(WorkspaceStatusStore);
    status.setBusy({ kind: 'import', label: 'Ouverture de 2 éléments…', jobId: null });
    expect(status.importing()).toBe(true);
    status.setBusy({ kind: 'op', label: 'Opening 2 items…' });
    expect(status.importing()).toBe(false);
    status.setBusy(null);
    expect(status.importing()).toBe(false);
    expect(status.busyLabel()).toBeNull();
  });
});

/** A UxBackend whose final progress the test sets. */
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

/** A running import job as `imt://jobs` reports it. */
function importJob(id: number, progress: Partial<JobInfo['progress']> = {}, patch: Partial<JobInfo> = {}): JobInfo {
  return {
    id,
    kind: 'import',
    title: 'Open /big',
    source: { window: 'main', link: null },
    state: 'running',
    progress: { fraction: 0.3, done: 3, total: 12, etaMs: 12_000, unit: 'items', bytesDone: 24_000_000, bytesTotal: 80_000_000, current: '/logs/web01/app.log', ...progress },
    message: null,
    queuedAt: 0,
    startedAt: 0,
    finishedAt: null,
    elapsedMs: 3000,
    etaMs: 11_000,
    cancellable: true,
    retryable: false,
    movable: false,
    position: null,
    resultType: null,
    error: null,
    unseen: false,
    retryOf: null,
    ...patch,
  };
}

describe('jobImportProgress', () => {
  it('reads files, bytes and the current file from the job, with defaults for what it does not report', () => {
    expect(jobImportProgress(importJob(1))).toEqual(p());
    const bare = importJob(2, { done: null, total: null, etaMs: null, bytesDone: undefined, bytesTotal: undefined, current: undefined }, { elapsedMs: 10 });
    expect(jobImportProgress(bare)).toEqual({ running: true, done: 0, total: 0, cancelled: false, bytesDone: 0, bytesTotal: 0, current: null, elapsedMs: 10, etaMs: 11_000 });
  });
});

describe('ImportProgressService and its widgets', () => {
  let ux: FakeUx;
  let release: (s: WorkspaceSummary) => void;

  function setup() {
    ux = new FakeUx();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), { provide: UxBackend, useValue: ux }] });
    const backend = TestBed.inject(ImtBackend);
    vi.spyOn(backend, 'openPaths').mockImplementation(() => new Promise<WorkspaceSummary>((r) => (release = r)));
    return { svc: TestBed.inject(ImportProgressService), store: TestBed.inject(WorkspaceStore), backend, jobs: TestBed.inject(JobsStore) };
  }

  afterEach(() => vi.useRealTimers());

  it('follows the import job, shows the status and the toast for long imports, and cancels', async () => {
    const { svc, store, jobs } = setup();
    await settle();
    const summary = store.summary()!;
    vi.useFakeTimers();
    const status = TestBed.createComponent(ImportStatus);
    const toast = TestBed.createComponent(ImportToast);
    const done = store.openPaths(['/big']);
    TestBed.tick();
    expect(svc.active()).toBe(true);
    expect(svc.progress()?.total).toBe(0);
    status.detectChanges();
    expect(text(status)).toContain('Scanning for files…');
    expect(text(status)).not.toContain('0 of 0');
    // The job's progress (imt://jobs) drives the widgets: no import_progress polling while it runs.
    jobs.apply({ jobs: [importJob(5, { done: 1 }, { kind: 'export', title: 'Export' }), importJob(7)], paused: [] });
    status.detectChanges();
    expect(text(status)).toContain('Importing 3 of 12 files · 23 MB of 76 MB · less than a minute left');
    expect(text(status)).toContain('app.log');
    const bar = status.nativeElement.querySelector('[role=progressbar]') as HTMLElement;
    expect(bar.getAttribute('aria-valuenow')).toBe('30');
    expect(ux.importProgress).not.toHaveBeenCalled();
    // The status-bar text opens the import's job in Jobs.
    const show = vi.spyOn(TestBed.inject(COMMAND_CONTEXT), 'showView');
    (status.nativeElement.querySelector('[data-testid=import-status-open]') as HTMLButtonElement).click();
    expect(show).toHaveBeenCalledWith('jobs');
    toast.detectChanges();
    expect(text(toast)).toBe('');
    await vi.advanceTimersByTimeAsync(LONG_IMPORT_MS);
    toast.detectChanges();
    // Over a workspace the status bar is the one progress surface: no card over the Explorer and the grid.
    expect(text(toast)).toBe('');
    TestBed.inject(WelcomeService).show();
    toast.detectChanges();
    expect(text(toast)).toContain('Reading app.log');
    buttonByText(toast, 'Cancel import').click();
    await vi.advanceTimersByTimeAsync(0);
    status.detectChanges();
    expect(ux.cancelled).toBe(1);
    expect(text(status)).toContain('Cancelling…');
    await svc.cancel();
    expect(ux.cancelled).toBe(1);
    // Finished with only some files read: one final read, and the banner says so.
    ux.progress = p({ running: false, done: 5, total: 12, cancelled: true, current: null });
    release(summary);
    await done;
    TestBed.tick();
    await vi.advanceTimersByTimeAsync(0);
    expect(svc.active()).toBe(false);
    expect(ux.importProgress).toHaveBeenCalledTimes(1);
    expect(svc.partial()).toEqual({ done: 5, total: 12 });
    vi.useRealTimers();
    const banner = TestBed.createComponent(PartialImportBanner);
    banner.detectChanges();
    expect(text(banner)).toContain('Partially imported: 5 of 12 files were read');
    buttonByText(banner, 'Dismiss').click();
    expect(svc.partial()).toBeNull();
  });

  it('follows the job of the busy state when it is known', async () => {
    const { svc, jobs } = setup();
    await settle();
    const status = TestBed.inject(WorkspaceStatusStore);
    status.setBusy({ kind: 'import', label: 'Applying the profile…', jobId: 3 });
    TestBed.tick();
    jobs.apply({ jobs: [importJob(3, { done: 2, current: '/logs/a.log' }), importJob(4, { done: 9 })], paused: [] });
    expect(svc.progress()).toMatchObject({ done: 2, current: '/logs/a.log' });
    // Its job is not listed (yet): the newest running import.
    status.setBusy({ kind: 'import', label: 'Applying the profile…', jobId: 99 });
    expect(svc.progress()?.done).toBe(9);
    status.setBusy(null);
    TestBed.tick();
    await settle();
    expect(svc.progress()).toBeNull();
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

  it('keeps going when the final progress cannot be read, reports a failed cancel and ignores cancel when idle', async () => {
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
});
