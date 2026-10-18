import { signal } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { Subject } from 'rxjs';
import { MemoryLogger } from '../../testing/helpers';
import { Query, WorkspaceSummary } from '../core/api.types';
import { Logger } from '../core/logger';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { ImtEvents } from '../extensions/events';
import { CommandContext } from '../extensions/tokens';
import { OverwriteGuard } from '../ui/overwrite-guard';
import { DiagnosticsCommands } from './diagnostics-commands';
import { diagnosticsFileName, DiagnosticsExport } from './diagnostics-export.service';
import { DiagnosticsReportService, DIAGNOSTICS_REPORT_VIEW } from './diagnostics-report.service';
import { DiagnosticsReportStore } from './diagnostics-report.store';
import { BREADCRUMB_WINDOW_MS, DiagnosticsReportPanel } from './diagnostics-report-view';
import { DiagnosticsReportState } from './diagnostics-report.types';
import { ExportTarget } from './export-target';
import { sampleReport, sampleReportState, SAMPLE_REPORT_IMAGE } from './mock/mock-diagnostics-report';
import { MockSnapshotBackend } from './mock/mock-snapshot-backend';
import { SnapshotBackend, TauriSnapshotBackend } from './snapshot-backend';
import { createSnapshotBackend } from './provide-diagnostics';

const ZIP = { path: '/samples/diagnostics/imt-diagnostics-sample.zip', name: 'imt-diagnostics-sample.zip' };

function summary(sources: { path: string; name: string }[]): WorkspaceSummary {
  return { sources } as unknown as WorkspaceSummary;
}

class FakeTarget extends ExportTarget {
  path: string | null = '/out/diag.zip';
  asked: { name: string; title: string }[] = [];
  override async pickZip(name: string, title = '') {
    this.asked.push({ name, title });
    return this.path;
  }
}

function setup(sources: { path: string; name: string }[] = [ZIP]) {
  const workspace = signal<WorkspaceSummary | null>(summary(sources));
  const backend = new MockSnapshotBackend(undefined, () => workspace());
  const changed = new Subject<WorkspaceSummary>();
  const ctx = {
    setQuery: vi.fn<(patch: Partial<Query>) => void>(),
    showView: vi.fn<(id: string) => void>(),
    notifyError: vi.fn<(m: string) => void>(),
  };
  const target = new FakeTarget();
  const overwrite = { write: vi.fn(async (_p: string, write: (o: boolean) => Promise<unknown>) => write(false)) };
  TestBed.configureTestingModule({
    providers: [
      { provide: SnapshotBackend, useValue: backend },
      { provide: Logger, useValue: new MemoryLogger() },
      { provide: COMMAND_CONTEXT, useValue: ctx as unknown as CommandContext },
      { provide: ImtEvents, useValue: { workspaceChanged: changed.asObservable(), workspace } },
      { provide: ExportTarget, useValue: target },
      { provide: OverwriteGuard, useValue: overwrite },
    ],
  });
  return { backend, workspace, changed, ctx, target, overwrite };
}

async function flush(f?: ComponentFixture<unknown>) {
  for (let i = 0; i < 4; i++) {
    await (f ? f.whenStable() : Promise.resolve());
    await new Promise((r) => setTimeout(r));
    f?.detectChanges();
  }
}

describe('mock diagnostics report', () => {
  it('is null without a diagnostics zip and a sample report with one', async () => {
    const none = setup([{ path: '/logs/app.log', name: 'app.log' }]);
    expect(await none.backend.diagnosticsReport()).toEqual({ report: null, notice: null });
    TestBed.resetTestingModule();
    const { backend } = setup();
    const { report } = await backend.diagnosticsReport();
    expect(report?.name).toBe('imt-diagnostics-sample.zip');
    expect(report?.processes.map((p) => p.kind)).toEqual(expect.arrayContaining(['ai-worker', 'llama-server', 'plugin']));
    const llama = report?.processes.find((p) => p.name === 'llama-server');
    expect([llama?.state, llama?.restarts, llama?.lastExit]).toEqual(['crashed', 1, 'exit code 3']);
    expect(report?.processes.some((p) => p.state === 'quarantined' && p.quarantined)).toBe(true);
    const id = report?.snapshots[0]?.id ?? '';
    expect(await backend.diagnosticsReportImage(id)).toBe(SAMPLE_REPORT_IMAGE);
    await expect(backend.diagnosticsReportImage('nope')).rejects.toThrow();
  });

  it('matches by name when the path does not and falls back to the path for the name', () => {
    expect(sampleReportState([{ path: '/x/y', name: 'IMT-Diagnostics-1.ZIP' }]).report?.name).toBe('IMT-Diagnostics-1.ZIP');
    expect(sampleReportState([{ path: 'C:\\x\\imt-diagnostics-2.zip', name: '' }]).report?.name).toBe('imt-diagnostics-2.zip');
    expect(sampleReport('a.zip').name).toBe('a.zip');
  });

  it('lists the new report files in the preview', async () => {
    const { backend } = setup([]);
    const names = (await backend.reportPreview('')).entries.map((e) => e.name);
    expect(names).toEqual(expect.arrayContaining(['manifest.json', 'processes.json', 'session-recovery.json', 'logs/children/ai-worker.log', 'logs/children/llama-server.log', 'logs/imt.log']));
  });

  it('is created by createSnapshotBackend with a lazy workspace lookup', async () => {
    const workspace = signal<WorkspaceSummary | null>(summary([ZIP]));
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useValue: new MemoryLogger() }, { provide: ImtEvents, useValue: { workspace } }] });
    const backend = TestBed.runInInjectionContext(() => createSnapshotBackend());
    expect((await backend.diagnosticsReport()).report?.name).toBe('imt-diagnostics-sample.zip');
  });
});

describe('TauriSnapshotBackend diagnostics report', () => {
  it('invokes the report commands', async () => {
    const invoke = vi.fn(async (_c: string, _a?: unknown) => 'x');
    const b = new TauriSnapshotBackend(invoke as never, vi.fn() as never);
    await b.diagnosticsReport();
    await b.diagnosticsReportImage('s1');
    expect(invoke.mock.calls).toEqual([['diagnostics_report', {}], ['diagnostics_report_image', { id: 's1' }]]);
  });
});

describe('DiagnosticsReportStore', () => {
  it('loads a report, keeps the selection while it is the same report, and clears it for another', async () => {
    const { backend, workspace } = setup();
    const store = TestBed.inject(DiagnosticsReportStore);
    expect(await store.refresh()).toMatchObject({ notice: null });
    await store.loadImages();
    expect(Object.keys(store.images())).toHaveLength(1);
    const id = store.report()?.snapshots[0]?.id ?? '';
    await store.show(id);
    expect(store.current()?.id).toBe(id);
    await store.refresh();
    expect(store.selected()).toBe(id);
    workspace.set(summary([{ path: '/x/imt-diagnostics-other.zip', name: 'imt-diagnostics-other.zip' }]));
    await store.refresh();
    expect(store.selected()).toBeNull();
    expect(store.images()).toEqual({});
    await store.show(null);
    workspace.set(summary([]));
    await store.refresh();
    expect(store.report()).toBeNull();
    expect(backend).toBeTruthy();
  });

  it('reports a failing request and a failing image', async () => {
    const { backend } = setup();
    const store = TestBed.inject(DiagnosticsReportStore);
    const state = vi.spyOn(backend, 'diagnosticsReport').mockRejectedValueOnce(new Error('boom'));
    expect(await store.refresh()).toBeNull();
    expect(store.error()).toContain('boom');
    expect(state).toHaveBeenCalled();
    await store.refresh();
    expect(store.error()).toBeNull();
    vi.spyOn(backend, 'diagnosticsReportImage').mockRejectedValueOnce(new Error('no image'));
    const id = store.report()?.snapshots[0]?.id ?? '';
    await store.show(id);
    expect(store.imageErrors()[id]).toContain('no image');
    await store.show(id);
    expect(store.images()[id]).toBe(SAMPLE_REPORT_IMAGE);
    expect(store.imageErrors()[id]).toBeUndefined();
    await store.show(id);
  });
});

describe('DiagnosticsReportPanel', () => {
  it('renders the sections of the report', async () => {
    setup();
    const f = TestBed.createComponent(DiagnosticsReportPanel);
    await flush(f);
    const el: HTMLElement = f.nativeElement;
    expect(el.querySelector('[data-testid=diag-header]')?.textContent).toContain('imt-diagnostics-sample.zip');
    expect(el.querySelector('[data-testid=diag-header]')?.textContent).toContain('0.9.0');
    expect(el.querySelector('[data-testid=diag-description]')?.textContent).toContain('stopped answering');
    expect(el.querySelector('[data-testid=diag-unreadable]')).toBeNull();
    expect(el.querySelector('[data-testid=diag-system]')?.textContent).toContain('x86_64');
    expect(el.querySelector('[data-testid=diag-system]')?.textContent).toContain('no');
    expect(el.querySelector('[data-testid=diag-plugins]')?.textContent).toContain('net.decryptology.web');
    const rows = [...el.querySelectorAll('[data-testid=diag-processes] tbody tr')];
    expect(rows).toHaveLength(3);
    expect(el.querySelector('[data-testid=diag-processes] th')?.textContent).toContain('Name');
    expect(rows[1]?.getAttribute('data-state')).toBe('crashed');
    expect(rows[1]?.textContent).toContain('exit code 3');
    expect(rows[2]?.textContent).toContain('failed 3 times in a row');
    expect(rows[2]?.querySelector('.state')?.classList).toContain('quarantined');
    const steps = [...el.querySelectorAll('[data-testid=diag-steps] li')].map((li) => li.textContent?.trim());
    expect(steps[0]).toBe('Opened <dir#1a2b3c4d>/samples (3 files) from the menu');
    expect(el.querySelectorAll('[data-testid=diag-breadcrumbs] button')).toHaveLength(2);
    expect(el.querySelectorAll('[data-testid=diag-snapshot-card]')).toHaveLength(0 + 1);
  });

  it('shows a warning for unreadable files and tolerates odd system info', async () => {
    setup();
    const store = TestBed.inject(DiagnosticsReportStore);
    const base = sampleReport('r.zip');
    vi.spyOn(TestBed.inject(SnapshotBackend), 'diagnosticsReport').mockResolvedValue({
      report: { ...base, unreadable: ['processes.json'], description: null, steps: [], breadcrumbs: [], snapshots: [], processes: [], systemInfo: { plugins: [3, { id: 1 }], cpuCores: 4, os: null } },
      notice: null,
    });
    await store.refresh();
    const f = TestBed.createComponent(DiagnosticsReportPanel);
    await flush(f);
    const el: HTMLElement = f.nativeElement;
    expect(el.querySelector('[data-testid=diag-unreadable]')?.textContent).toContain('processes.json');
    expect(el.querySelector('[data-testid=diag-description]')).toBeNull();
    expect(el.textContent).toContain('No activity was recorded.');
    expect(el.textContent).toContain('No child processes were recorded.');
    expect(el.textContent).toContain('No breadcrumbs were recorded.');
    expect(el.textContent).toContain('The report has no error snapshots.');
    expect(el.querySelector('[data-testid=diag-system]')?.textContent).toContain('4');
  });

  it('shows the empty state when no report is open', async () => {
    setup([]);
    const f = TestBed.createComponent(DiagnosticsReportPanel);
    await flush(f);
    expect(f.nativeElement.querySelector('[data-testid=diag-empty]')?.textContent).toContain('No diagnostics report is open.');
  });

  it('shows a failure inline and loads again on Retry', async () => {
    const { backend } = setup();
    vi.spyOn(backend, 'diagnosticsReport').mockRejectedValueOnce(new Error('cannot read'));
    const f = TestBed.createComponent(DiagnosticsReportPanel);
    await flush(f);
    const el: HTMLElement = f.nativeElement;
    expect(el.querySelector('[data-testid=diag-error]')?.textContent).toContain('cannot read');
    expect(el.querySelector('[data-testid=diag-empty]')).toBeNull();
    (el.querySelector('[data-testid=diag-error] button') as HTMLButtonElement).click();
    await flush(f);
    expect(el.querySelector('[data-testid=diag-error]')).toBeNull();
    expect(el.querySelector('[data-testid=diag-processes]')).not.toBeNull();
  });

  it('zooms the timeline around a breadcrumb', async () => {
    const { ctx } = setup();
    const f = TestBed.createComponent(DiagnosticsReportPanel);
    await flush(f);
    const buttons = f.nativeElement.querySelectorAll('[data-testid=diag-breadcrumbs] button') as NodeListOf<HTMLButtonElement>;
    buttons[1]?.click();
    const at = Date.parse('2026-10-14T09:29:30Z');
    expect(ctx.setQuery).toHaveBeenCalledWith({ from: at - BREADCRUMB_WINDOW_MS, to: at + BREADCRUMB_WINDOW_MS });
  });

  it('ignores a breadcrumb whose time is not a date', async () => {
    const { ctx } = setup();
    const store = TestBed.inject(DiagnosticsReportStore);
    const base = sampleReport('r.zip');
    vi.spyOn(TestBed.inject(SnapshotBackend), 'diagnosticsReport').mockResolvedValue({
      report: { ...base, breadcrumbs: [{ time: 'later', operation: 'x', trigger: 'menu', window: 'main' }] },
      notice: null,
    });
    await store.refresh();
    const f = TestBed.createComponent(DiagnosticsReportPanel);
    await flush(f);
    (f.nativeElement.querySelector('[data-testid=diag-breadcrumbs] button') as HTMLButtonElement).click();
    expect(ctx.setQuery).not.toHaveBeenCalled();
    expect(f.nativeElement.textContent).toContain('later');
  });

  it('opens a snapshot with its image and details, and closes it', async () => {
    setup();
    const f = TestBed.createComponent(DiagnosticsReportPanel);
    await flush(f);
    const el: HTMLElement = f.nativeElement;
    expect(el.querySelector('[data-testid=diag-snapshot-card] img')?.getAttribute('src')).toBe(SAMPLE_REPORT_IMAGE);
    (el.querySelector('[data-testid=diag-snapshot-card] button') as HTMLButtonElement).click();
    await flush(f);
    const detail = el.querySelector('[data-testid=diag-snapshot-detail]');
    expect(detail?.textContent).toContain('engine-error');
    expect(detail?.textContent).toContain('exit code 3');
    expect(detail?.textContent).toContain('AI chat');
    expect(detail?.querySelector('[data-testid=diag-snapshot-image]')?.getAttribute('src')).toBe(SAMPLE_REPORT_IMAGE);
    expect(detail?.querySelector('[data-testid=diag-snapshot-steps] li')?.textContent?.trim()).toBe('Opened <dir#1a2b3c4d>/samples (3 files) from the menu');
    ((detail as HTMLElement).querySelector('button.ghost') as HTMLButtonElement).click();
    await flush(f);
    expect(el.querySelector('[data-testid=diag-snapshot-detail]')).toBeNull();
  });

  it('shows why a snapshot image could not be loaded', async () => {
    const { backend } = setup();
    const f = TestBed.createComponent(DiagnosticsReportPanel);
    await flush(f);
    TestBed.inject(DiagnosticsReportStore);
    vi.spyOn(backend, 'diagnosticsReportImage').mockRejectedValue(new Error('image gone'));
    const store = TestBed.inject(DiagnosticsReportStore);
    // Drop the cached thumbnail so the large view has to load it again.
    await store.refresh();
    const other = sampleReport('imt-diagnostics-sample.zip');
    vi.spyOn(backend, 'diagnosticsReport').mockResolvedValue({ report: { ...other, snapshots: other.snapshots.map((s) => ({ ...s, id: 'fresh' })) }, notice: null });
    await store.refresh();
    await store.show('fresh');
    await flush(f);
    expect(f.nativeElement.querySelector('[data-testid=diag-snapshot-detail] [role=alert]')?.textContent).toContain('image gone');
  });
});

describe('DiagnosticsReportService', () => {
  it('shows the view when a report appears, once per report, and notifies a notice once', async () => {
    const { ctx, changed, workspace, backend } = setup([]);
    const svc = TestBed.inject(DiagnosticsReportService);
    svc.start();
    svc.start();
    changed.next(summary([]));
    await flush();
    expect(ctx.showView).not.toHaveBeenCalled();

    workspace.set(summary([ZIP]));
    changed.next(summary([ZIP]));
    await flush();
    expect(ctx.showView).toHaveBeenCalledTimes(1);
    expect(ctx.showView).toHaveBeenCalledWith(DIAGNOSTICS_REPORT_VIEW);
    expect(TestBed.inject(DiagnosticsReportStore).report()?.name).toBe('imt-diagnostics-sample.zip');
    expect(Object.keys(TestBed.inject(DiagnosticsReportStore).images())).toHaveLength(1);

    changed.next(summary([ZIP]));
    await flush();
    expect(ctx.showView).toHaveBeenCalledTimes(1);

    workspace.set(summary([{ path: '/b/imt-diagnostics-b.zip', name: 'imt-diagnostics-b.zip' }]));
    changed.next(summary([]));
    await flush();
    expect(ctx.showView).toHaveBeenCalledTimes(2);

    const withNotice: DiagnosticsReportState = { report: null, notice: 'That zip could not be read as a report.' };
    vi.spyOn(backend, 'diagnosticsReport').mockResolvedValue(withNotice);
    changed.next(summary([]));
    await flush();
    changed.next(summary([]));
    await flush();
    expect(ctx.notifyError).toHaveBeenCalledTimes(1);
    expect(ctx.notifyError).toHaveBeenCalledWith(withNotice.notice);
  });

  it('logs and swallows failures', async () => {
    const { changed, backend, ctx } = setup();
    const logger = TestBed.inject(Logger) as MemoryLogger;
    TestBed.inject(DiagnosticsReportService).start();
    vi.spyOn(backend, 'diagnosticsReport').mockRejectedValueOnce(new Error('down'));
    changed.next(summary([]));
    await flush();
    expect(ctx.showView).not.toHaveBeenCalled();
    expect(TestBed.inject(DiagnosticsReportStore).error()).toContain('down');

    const svc = TestBed.inject(DiagnosticsReportService);
    vi.spyOn(TestBed.inject(DiagnosticsReportStore), 'refresh').mockRejectedValueOnce(new Error('odd'));
    await svc.check();
    expect(logger.entries.some((e) => e.message.includes('odd'))).toBe(true);
  });
});

describe('DiagnosticsExport', () => {
  it('names the file with the local time', () => {
    expect(diagnosticsFileName(new Date(2026, 9, 5, 7, 8, 9))).toBe('imt-diagnostics-20261005-070809.zip');
  });

  it('saves the report without a description and says where', async () => {
    const { backend, target } = setup();
    const save = vi.spyOn(backend, 'saveReport');
    const exporter = TestBed.inject(DiagnosticsExport);
    await exporter.save();
    expect(save).toHaveBeenCalledWith('/out/diag.zip', '', false);
    expect(target.asked[0]?.name).toMatch(/^imt-diagnostics-\d{8}-\d{6}\.zip$/);
    expect(target.asked[0]?.title).toBe('Save diagnostics');
    expect(exporter.status()?.kind).toBe('saved');
    expect(exporter.status()?.text).toMatch(/^Saved diagnostics \(\d+ files\) to \/out\/diag\.zip$/);
    expect(exporter.busy()).toBe(false);
    exporter.dismiss();
    expect(exporter.status()).toBeNull();
  });

  it('does nothing when the dialog is cancelled', async () => {
    const { backend, target } = setup();
    target.path = null;
    const save = vi.spyOn(backend, 'saveReport');
    const exporter = TestBed.inject(DiagnosticsExport);
    await exporter.save();
    expect(save).not.toHaveBeenCalled();
    expect(exporter.status()).toBeNull();
  });

  it('shows nothing when the user declines to replace a file', async () => {
    const { overwrite } = setup();
    overwrite.write.mockResolvedValueOnce(null);
    const exporter = TestBed.inject(DiagnosticsExport);
    await exporter.save();
    expect(exporter.status()).toBeNull();
  });

  it('reports a failure', async () => {
    const { backend } = setup();
    vi.spyOn(backend, 'saveReport').mockRejectedValue(new Error('disk full'));
    const exporter = TestBed.inject(DiagnosticsExport);
    await exporter.save();
    expect(exporter.status()?.kind).toBe('failed');
    expect(exporter.status()?.text).toContain('Could not save the diagnostics: disk full');
  });

  it('ignores a second save while one is running', async () => {
    const { target } = setup();
    const exporter = TestBed.inject(DiagnosticsExport);
    const first = exporter.save();
    await exporter.save();
    await first;
    expect(target.asked).toHaveLength(1);
  });

  it('is offered as the Save diagnostics command', async () => {
    const { backend } = setup();
    const save = vi.spyOn(backend, 'saveReport');
    const cmd = TestBed.inject(DiagnosticsCommands).commands.find((c) => c.id === 'diagnostics.saveDiagnostics');
    expect(cmd?.title).toBe('Save diagnostics…');
    expect(cmd?.category).toBe('Help');
    await cmd?.run(TestBed.inject(COMMAND_CONTEXT));
    expect(save).toHaveBeenCalled();
  });
});
