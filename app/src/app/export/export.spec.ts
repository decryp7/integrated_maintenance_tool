import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { ExporterInfo } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { Platform } from '../core/platform';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { ExportDialog, exportQuery } from './export-dialog';
import { ExportService, exporterForPath } from './export.service';
import { provideMockShare } from '../share/redaction/share-testing';
import { ShareBackend } from '../share/share-backend';
import { at } from '../../testing/must';
import { mockJobs } from '../jobs/mock-jobs';
import { mockUnsupported } from '../core/mock/mock-unsupported';
import { NotificationsStore } from '../notifications/notifications.store';

const exporters: ExporterInfo[] = [
  { id: 'csv', name: 'CSV', extension: 'csv' },
  { id: 'clef', name: 'CLEF', extension: 'clef' },
];

describe('exporterForPath', () => {
  it('matches the extension case-insensitively and falls back to the first exporter', () => {
    expect(exporterForPath('/x/events.CLEF', exporters)?.id).toBe('clef');
    expect(exporterForPath('/x/events.txt', exporters)?.id).toBe('csv');
    expect(exporterForPath('/x/noext', exporters)?.id).toBe('csv');
    expect(exporterForPath('/x/a.csv', [])).toBeNull();
  });
});

describe('exportQuery', () => {
  const current = { ...EMPTY_QUERY, text: 'boom', from: 1, to: 2, groupBy: ['A'] };
  it('keeps, relaxes or drops the filter', () => {
    expect(exportQuery(current, 'filter')).toBe(current);
    expect(exportQuery(current, 'filterAnyTime')).toMatchObject({ text: 'boom', from: null, to: null });
    expect(exportQuery(current, 'all')).toEqual({ ...EMPTY_QUERY, groupBy: ['A'] });
  });
});

describe('ExportDialog', () => {
  const closed = vi.fn();
  const pick = vi.fn<(name: string, formats: ExporterInfo[]) => Promise<string | null>>();

  async function setup() {
    closed.mockClear();
    pick.mockReset();
    TestBed.configureTestingModule({
      providers: [provideTinyBackend(), provideMockShare(), { provide: Platform, useValue: { isTauri: () => false, pickPaths: async () => [], onFilesDropped: async () => () => undefined, pickExportPath: pick } }, { provide: DIALOG_REF, useValue: { close: closed, closed: Promise.resolve(undefined) } }],
    });
    const store = TestBed.inject(WorkspaceStore);
    await settle();
    const fixture = TestBed.createComponent(ExportDialog);
    await settle(fixture);
    return { fixture, store, backend: TestBed.inject(ImtBackend) };
  }

  it('shows formats and the number of events, and recounts when the scope changes', async () => {
    const { fixture, store } = await setup();
    expect(all(fixture, '#exp-format option').map((o) => o.textContent?.trim())).toEqual(['CSV (.csv)', 'Compact Log Event Format (.clef)']);
    expect(text(fixture)).toContain('11 events will be exported');
    store.setTextInput('boom');
    await new Promise((r) => setTimeout(r, 300));
    await settle(fixture);
    const filtered = text(fixture);
    expect(filtered).not.toContain('11 events');
    el<HTMLInputElement>(fixture, 'input[value=all]').click();
    await settle(fixture);
    expect(text(fixture)).toContain('11 events will be exported');
    expect(fixture.nativeElement.querySelector('input[value=filterAnyTime]')).toBeNull();
    store.setRange(store.summary()!.from!, store.summary()!.from! + 1000);
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('input[value=filterAnyTime]')).not.toBeNull();
  });

  it('starts the export as a job, tells the user it runs in Jobs and closes at once', async () => {
    const { fixture, backend } = await setup();
    const spy = vi.spyOn(backend, 'exportEvents');
    pick.mockResolvedValue('/out/events.clef');
    el<HTMLInputElement>(fixture, 'input[value=desc]').click();
    el<HTMLSelectElement>(fixture, '#exp-format').value = 'clef';
    el<HTMLSelectElement>(fixture, '#exp-format').dispatchEvent(new Event('change'));
    buttonByText(fixture, 'Export…').click();
    await settle(fixture);
    expect(at(pick.mock.calls, 0)[0]).toBe('events.clef');
    expect(at(spy.mock.calls, 0)[0]).toMatchObject({ format: 'clef', path: '/out/events.clef', descending: true });
    // The backend call returned the job id at once; the dialog neither waits for the export nor shows a result.
    await expect(at(spy.mock.results, 0).value).resolves.toEqual(expect.any(Number));
    const toast = at(TestBed.inject(NotificationsStore).toasts(), 0);
    expect([toast.severity, toast.title, toast.primary]).toEqual(['info', 'Exporting to events.clef — running in Jobs', { type: 'showJobs' }]);
    expect(closed).toHaveBeenCalled();
    // The job writes the file and its message says how many events went out; Show in folder reveals it.
    const end = await mockJobs().wait(await at(spy.mock.results, 0).value);
    expect([end.state, end.message, end.result]).toEqual(['done', 'Exported 11 events.', { type: 'file', path: '/out/events.clef' }]);
  });

  it('lets the file extension override the picked format', async () => {
    const { fixture, backend } = await setup();
    const spy = vi.spyOn(backend, 'exportEvents');
    pick.mockResolvedValue('/out/x.clef');
    buttonByText(fixture, 'Export…').click();
    await settle(fixture);
    expect(at(spy.mock.calls, 0)[0].format).toBe('clef');
  });

  it('does nothing when the save dialog is cancelled and shows backend errors', async () => {
    const { fixture, backend } = await setup();
    pick.mockResolvedValueOnce(null);
    buttonByText(fixture, 'Export…').click();
    await settle(fixture);
    expect(closed).not.toHaveBeenCalled();
    pick.mockResolvedValueOnce('/out/e.csv');
    vi.spyOn(backend, 'exportEvents').mockRejectedValueOnce(new Error('disk full'));
    buttonByText(fixture, 'Export…').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('disk full');
    pick.mockResolvedValueOnce('/out/e.csv');
    vi.spyOn(backend, 'exportEvents').mockRejectedValueOnce('plain');
    buttonByText(fixture, 'Export…').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('plain');
    buttonByText(fixture, 'Cancel').click();
    expect(closed).toHaveBeenCalled();
  });

  it('reports count failures and disables export when nothing matches', async () => {
    const { fixture, store } = await setup();
    store.setTextInput('zzz-no-match');
    await new Promise((r) => setTimeout(r, 300));
    await settle(fixture);
    expect(buttonByText(fixture, 'Export…').disabled).toBe(true);
    vi.spyOn(TestBed.inject(ImtBackend), 'queryPage').mockRejectedValueOnce('count failed');
    el<HTMLInputElement>(fixture, 'input[value=all]').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('count failed');
  });
});

describe('ExportDialog redaction', () => {
  async function setup(onExport: boolean, summaryUnsupported = false) {
    const backend = tinyBackend();
    if (onExport) await mockJobs().wait(await backend.saveProfile((await backend.getProfile()).toml.replace('on_export = false', 'on_export = true')));
    const pick = vi.fn(async () => '/out/events.csv');
    TestBed.configureTestingModule({
      providers: [provideTinyBackend(backend), provideMockShare(), { provide: Platform, useValue: { isTauri: () => false, pickPaths: async () => [], onFilesDropped: async () => () => undefined, pickExportPath: pick } }, { provide: DIALOG_REF, useValue: { close: () => undefined, closed: Promise.resolve(undefined) } }],
    });
    const store = TestBed.inject(WorkspaceStore);
    // An engine that cannot summarise the redaction (older stores).
    if (summaryUnsupported) vi.spyOn(TestBed.inject(ShareBackend), 'redactionSummary').mockRejectedValue(mockUnsupported('the redaction summary is not available yet'));
    await settle();
    const fixture = TestBed.createComponent(ExportDialog);
    await settle(fixture);
    return { fixture, store, backend };
  }

  const redactBox = (fixture: { nativeElement: HTMLElement }) => fixture.nativeElement.querySelector<HTMLInputElement>('input[type=checkbox]')!;

  it('says the preview is not available yet when the log store cannot summarise the redaction', async () => {
    const { fixture } = await setup(true, true);
    await settle(fixture);
    expect(text(fixture)).toContain('A preview of what will be redacted is not available yet');
    expect(fixture.nativeElement.querySelector('[role=alert]')).toBeNull();
  });

  it('starts from the profile default and previews what will be masked', async () => {
    const { fixture } = await setup(true);
    expect(redactBox(fixture).checked).toBe(true);
    await settle(fixture);
    expect(text(fixture)).toMatch(/\d+ of 11 events change/);
    redactBox(fixture).click();
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('imt-redaction-summary')).toBeNull();
  });

  it('exports redacted: the job message says how many values were masked', async () => {
    const { fixture, backend } = await setup(false);
    expect(redactBox(fixture).checked).toBe(false);
    redactBox(fixture).click();
    await settle(fixture);
    const spy = vi.spyOn(backend, 'exportEvents');
    buttonByText(fixture, 'Export…').click();
    await settle(fixture);
    expect(at(spy.mock.calls, 0)[0].redact).toBe(true);
    expect(at(TestBed.inject(NotificationsStore).toasts(), 0).title).toBe('Exporting to events.csv — running in Jobs');
    const end = await mockJobs().wait(await at(spy.mock.results, 0).value);
    expect(end.message).toMatch(/^Exported 11 events \(redacted, \d+ values masked\)\.$/);
  });

  it('keeps the user\'s choice when the profile default arrives later', async () => {
    const backend = tinyBackend();
    await mockJobs().wait(await backend.saveProfile((await backend.getProfile()).toml.replace('on_export = false', 'on_export = true')));
    TestBed.configureTestingModule({
      providers: [provideTinyBackend(backend), provideMockShare(), { provide: DIALOG_REF, useValue: { close: () => undefined, closed: Promise.resolve(undefined) } }],
    });
    const fixture = TestBed.createComponent(ExportDialog);
    fixture.detectChanges();
    redactBox(fixture).click();
    redactBox(fixture).click();
    await settle(fixture);
    expect(redactBox(fixture).checked).toBe(false);
  });

  it('shows a failing preview', async () => {
    const { fixture } = await setup(false);
    const share = TestBed.inject(ShareBackend);
    vi.spyOn(share, 'redactionSummary').mockRejectedValueOnce(new Error('no rules')).mockRejectedValueOnce('plain');
    redactBox(fixture).click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('no rules');
    el<HTMLInputElement>(fixture, 'input[value=all]').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('plain');
  });
});

describe('ExportService', () => {
  it('opens the dialog, and refuses without exporters', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const store = TestBed.inject(WorkspaceStore);
    const service = TestBed.inject(ExportService);
    await expect(service.exportCurrent()).rejects.toThrow('No export formats');
    await settle();
    expect(store.summary()?.exporters).toHaveLength(2);
    const dialogs = TestBed.inject(DialogService);
    const done = service.exportCurrent();
    expect(at(dialogs.stack(), 0).options.title).toBe('Export events');
    dialogs.dismissTop();
    await done;
    expect(dialogs.stack()).toHaveLength(0);
  });
});
