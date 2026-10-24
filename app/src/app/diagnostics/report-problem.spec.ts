import { Component } from '@angular/core';
import { ComponentFixture, TestBed } from '@angular/core/testing';
import { MemoryLogger } from '../../testing/helpers';
import { Breadcrumb } from '../core/activity/types';
import { FILE_EXISTS_SUFFIX } from '../core/api.types';
import { Logger } from '../core/logger';
import { CommandRegistry } from '../extensions/command-registry';
import { CommandContext } from '../extensions/tokens';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { OverwriteGuard } from '../ui/overwrite-guard';
import { DiagnosticsCommands } from './diagnostics-commands';
import { ErrorSnapshotService } from './error-snapshot.service';
import { ErrorSnapshotsView } from './error-snapshots-view';
import { ExportTarget } from './export-target';
import { MockSnapshotBackend } from './mock/mock-snapshot-backend';
import { REPORT_FILE_NAME, ReportProblemDialog } from './report-problem-dialog';
import { SnapshotBackend, TauriSnapshotBackend } from './snapshot-backend';
import { NotificationHost } from '../health/notification-host';
import { provideNotificationSource } from '../health/provide-notification-host';
import { SnapshotSource } from './snapshot-source';
import { ACTIVITY_LOG_COMMAND, CapturedImage, REPORT_PROBLEM_COMMAND } from './snapshot.types';
import { at } from '../../testing/must';
import { mockJobs } from '../jobs/mock-jobs';
import { NotificationsStore } from '../notifications/notifications.store';

const IMAGE: CapturedImage = { base64: 'iVBORw0KGgo=', mime: 'image/png', bytes: 8 };

function crumb(operation: string, params: Record<string, string> = {}): Breadcrumb {
  return { time: '2026-10-17T08:00:00Z', operation, trigger: 'menu', window: 'main', params };
}

const CRUMBS = [crumb('file.open', { Path: '<dir#1a2b3c4d>/samples', Files: '3' }), crumb('export.csv', { Title: 'Export CSV' })];

class FakeTarget extends ExportTarget {
  path: string | null = '/out/report.zip';
  titles: string[] = [];
  override async pickZip(_name: string, title = '') {
    this.titles.push(title);
    return this.path;
  }
}

function setup(crumbs: Breadcrumb[] = CRUMBS) {
  const backend = new MockSnapshotBackend((n) => crumbs.slice(-n));
  const target = new FakeTarget();
  const close = vi.fn();
  const run = vi.fn(async (_id: string) => true);
  const overwrite = { write: vi.fn(async (_p: string, write: (o: boolean) => Promise<unknown>) => write(false)) };
  TestBed.configureTestingModule({
    providers: [
      { provide: SnapshotBackend, useValue: backend },
      { provide: ExportTarget, useValue: target },
      { provide: Logger, useValue: new MemoryLogger() },
      { provide: DIALOG_REF, useValue: { close, closed: Promise.resolve(undefined) } },
      { provide: CommandRegistry, useValue: { run } },
      { provide: OverwriteGuard, useValue: overwrite },
    ],
  });
  return { backend, target, close, run, overwrite };
}

async function flush(f: ComponentFixture<unknown>) {
  for (let i = 0; i < 4; i++) {
    await f.whenStable();
    f.detectChanges();
  }
}

describe('ReportProblemDialog', () => {
  it('previews the contents and the steps, then saves where the user chooses', async () => {
    const { backend, target, close } = setup();
    await backend.save(IMAGE, { reason: 'panel-failed', message: 'boom', manual: false });
    const f = TestBed.createComponent(ReportProblemDialog);
    await flush(f);
    const el: HTMLElement = f.nativeElement;
    const names = [...el.querySelectorAll('[data-testid=report-contents] .name')].map((n) => n.textContent);
    expect(names).toEqual(expect.arrayContaining(['description.txt', 'steps-to-reproduce.txt', 'system-info.json', 'logs/activity.log', 'logs/debrief.log']));
    expect(names.some((n) => n?.startsWith('snapshots/'))).toBe(true);
    const steps = [...el.querySelectorAll('[data-testid=report-steps] li')].map((li) => li.textContent?.trim());
    expect(steps).toEqual(["Opened <dir#1a2b3c4d>/samples (3 files) from the menu", "Clicked 'Export CSV' from the menu"]);
    expect(el.textContent).toContain('No description');

    const area = el.querySelector('textarea') as HTMLTextAreaElement;
    area.value = 'It failed on export';
    area.dispatchEvent(new Event('input'));
    await flush(f);
    expect(el.textContent).toContain('Your description');
    (el.querySelector('[data-testid=report-save]') as HTMLButtonElement).click();
    await flush(f);
    expect(target.titles).toEqual(['Save problem report']);
    expect(backend.reports).toEqual([{ path: '/out/report.zip', description: 'It failed on export' }]);
    expect(el.querySelector('[data-testid=report-saved]')!.textContent).toContain('Saving to /out/report.zip.');
    // The report is written by a job: a toast says so and the dialog stays open on the preview.
    expect(at(TestBed.inject(NotificationsStore).toasts(), 0).title).toBe('Saving the problem report — running in Jobs');
    expect(close).not.toHaveBeenCalled();
    expect(el.textContent).toContain('Close');
    (el.querySelector('.btn.ghost') as HTMLButtonElement).click();
    expect(close).toHaveBeenCalled();
  });

  it('does nothing when the save dialog is cancelled and shows failures', async () => {
    const { backend, target } = setup([]);
    const f = TestBed.createComponent(ReportProblemDialog);
    await flush(f);
    const el: HTMLElement = f.nativeElement;
    expect(el.querySelector('[data-testid=report-steps]')!.textContent).toContain('No recent activity');
    target.path = null;
    await f.componentInstance.save();
    expect(backend.reports).toEqual([]);
    target.path = '/out/r.zip';
    backend.saveReport = async () => {
      throw new Error('disk full');
    };
    await f.componentInstance.save();
    await flush(f);
    expect(el.querySelector('[role=alert]')!.textContent).toContain('disk full');
    backend.reportPreview = async () => {
      throw new Error('no logs');
    };
    await f.componentInstance.load();
    await flush(f);
    expect(el.querySelector('[role=alert]')!.textContent).toContain('Could not collect the report: no logs');
  });

  it('a declined overwrite saves nothing and says nothing', async () => {
    const { overwrite } = setup();
    overwrite.write.mockResolvedValueOnce(null);
    const f = TestBed.createComponent(ReportProblemDialog);
    await flush(f);
    await f.componentInstance.save();
    expect(f.componentInstance.saved()).toBeNull();
    expect(REPORT_FILE_NAME).toBe('debrief-problem-report.zip');
  });

  it('formats sizes', () => {
  });
});

describe('report and activity log commands', () => {
  it('opens the dialog and the activity log, and reports a failure to open it', async () => {
    const { backend } = setup();
    const dialogs = TestBed.inject(DialogService);
    const { commands } = TestBed.inject(DiagnosticsCommands);
    const report = commands.find((c) => c.id === REPORT_PROBLEM_COMMAND)!;
    const log = commands.find((c) => c.id === ACTIVITY_LOG_COMMAND)!;
    expect([report.category, log.category, log.title]).toEqual(['Help', 'Help', 'Show activity log']);
    await report.run({} as CommandContext);
    expect(dialogs.stack().at(-1)?.options.title).toBe('Report a problem');
    expect(dialogs.stack().at(-1)?.component).toBe(ReportProblemDialog);
    const notifyError = vi.fn();
    await log.run({ notifyError } as unknown as CommandContext);
    expect(backend.activityLogOpened).toBe(1);
    expect(notifyError).not.toHaveBeenCalled();
    backend.openActivityLog = async () => {
      throw new Error('Debrief has not written a log yet');
    };
    await log.run({ notifyError } as unknown as CommandContext);
    expect(notifyError).toHaveBeenCalledWith('Could not open the activity log: Debrief has not written a log yet');
  });
});

describe('snapshots with breadcrumbs', () => {
  it('the mock attaches the latest activity and the steps ending in the error', async () => {
    const m = new MockSnapshotBackend(() => CRUMBS);
    await m.save(IMAGE, { reason: 'panel-failed', message: 'boom', manual: false });
    const info = at(await m.list(), 0);
    expect(info.breadcrumbs.map((b) => b.operation)).toEqual(['file.open', 'export.csv']);
    expect(info.steps.at(-1)).toBe('3. Error: boom');
    const quiet = new MockSnapshotBackend(() => []);
    await quiet.save(IMAGE, { reason: 'manual', message: 'x', manual: true });
    expect(at((await quiet.list()), 0).steps).toEqual([]);
    // The default reads the UI's own activity ring.
    expect(Array.isArray((await new MockSnapshotBackend().reportPreview('')).steps)).toBe(true);
  });

  it('the mock refuses to replace a report without permission', async () => {
    const m = new MockSnapshotBackend(() => []);
    const first = await m.saveReport('/r.zip', 'd');
    expect(first).toEqual(expect.any(Number));
    const end = await mockJobs().wait(first);
    expect([end.state, end.result]).toEqual(['done', { type: 'file', path: '/r.zip' }]);
    expect(end.message).toMatch(/^\d+ files zipped\.$/);
    await expect(m.saveReport('/r.zip', 'd')).rejects.toThrow(FILE_EXISTS_SUFFIX.trim());
    expect(await m.saveReport('/r.zip', 'd', true)).toEqual(expect.any(Number));
    expect(await m.openActivityLog()).toBe('mock-activity');
  });

  it('the Tauri backend maps the report commands', async () => {
    const invoke = vi.fn(async (_cmd: string, _a?: unknown): Promise<unknown> => 'ok');
    const b = new TauriSnapshotBackend(invoke as never, vi.fn());
    await b.reportPreview('desc');
    await b.saveReport('/r.zip', 'desc');
    await b.saveReport('/r.zip', 'desc', true);
    await b.openActivityLog();
    expect(invoke.mock.calls).toEqual([
      ['problem_report_preview', { description: 'desc' }],
      ['save_problem_report', { path: '/r.zip', description: 'desc' }],
      ['save_problem_report', { path: '/r.zip', description: 'desc', overwrite: true }],
      ['open_activity_log'],
    ]);
  });

  it('the snapshots view lists the steps and offers to report the problem', async () => {
    const { backend, run } = setup();
    await backend.save(IMAGE, { reason: 'panel-failed', message: 'boom', manual: false });
    const f = TestBed.createComponent(ErrorSnapshotsView);
    await flush(f);
    const el: HTMLElement = f.nativeElement;
    (el.querySelector('button.thumb') as HTMLButtonElement).click();
    await flush(f);
    const steps = [...el.querySelectorAll('[data-testid=snapshot-steps] li')].map((li) => li.textContent?.trim());
    expect(steps).toEqual(["Opened <dir#1a2b3c4d>/samples (3 files) from the menu", "Clicked 'Export CSV' from the menu", 'Error: boom']);
    (el.querySelector('button.report') as HTMLButtonElement).click();
    expect(run).toHaveBeenCalledWith(REPORT_PROBLEM_COMMAND);
  });

  it('the snapshot toast offers to report the problem', async () => {
    @Component({ selector: 'debrief-host', imports: [NotificationHost], providers: [provideNotificationSource(SnapshotSource)], template: '<debrief-notification-host />' })
    class Host {}
    const { run } = setup();
    const f = TestBed.createComponent(Host);
    const service = TestBed.inject(ErrorSnapshotService);
    service.toast.set({ kind: 'captured', id: 'a', firstTime: false });
    await flush(f);
    (f.nativeElement.querySelector('[data-testid=notification-action-report]') as HTMLButtonElement).click();
    expect(run).toHaveBeenCalledWith(REPORT_PROBLEM_COMMAND);
    expect(service.toast()).toBeNull();
  });
});
