import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text } from '../../testing/helpers';
import { ExporterInfo } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { Platform } from '../core/platform';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { ExportDialog, exportQuery } from './export-dialog';
import { ExportService, exporterForPath } from './export.service';

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
      providers: [provideTinyBackend(), { provide: Platform, useValue: { isTauri: () => false, pickPaths: async () => [], onFilesDropped: async () => () => undefined, pickExportPath: pick } }, { provide: DIALOG_REF, useValue: { close: closed, closed: Promise.resolve(undefined) } }],
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

  it('exports with the chosen format and order, reports it and closes', async () => {
    const { fixture, store, backend } = await setup();
    const spy = vi.spyOn(backend, 'exportEvents');
    pick.mockResolvedValue('/out/events.clef');
    el<HTMLInputElement>(fixture, 'input[value=desc]').click();
    el<HTMLSelectElement>(fixture, '#exp-format').value = 'clef';
    el<HTMLSelectElement>(fixture, '#exp-format').dispatchEvent(new Event('change'));
    buttonByText(fixture, 'Export…').click();
    await settle(fixture);
    expect(pick.mock.calls[0][0]).toBe('events.clef');
    expect(spy.mock.calls[0][0]).toMatchObject({ format: 'clef', path: '/out/events.clef', descending: true });
    expect(store.notice()).toBe('Exported 11 events to events.clef');
    expect(closed).toHaveBeenCalled();
  });

  it('lets the file extension override the picked format', async () => {
    const { fixture, backend } = await setup();
    const spy = vi.spyOn(backend, 'exportEvents');
    pick.mockResolvedValue('/out/x.clef');
    buttonByText(fixture, 'Export…').click();
    await settle(fixture);
    expect(spy.mock.calls[0][0].format).toBe('clef');
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
    expect(dialogs.stack()[0].options.title).toBe('Export events');
    dialogs.dismissTop();
    await done;
    expect(dialogs.stack()).toHaveLength(0);
  });
});
