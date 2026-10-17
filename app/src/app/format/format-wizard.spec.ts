import { Type } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { ParsePreviewRequest } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { DIALOG_REF } from '../ui/dialog.service';
import { FormatWizard } from './format-wizard';

/** Selects `needle` inside the element's text, like a user dragging over it. */
function selectText(root: HTMLElement, needle: string): void {
  const node = root.firstChild as Text;
  const start = node.data.indexOf(needle);
  const range = document.createRange();
  range.setStart(node, start);
  range.setEnd(node, start + needle.length);
  const sel = document.getSelection()!;
  sel.removeAllRanges();
  sel.addRange(range);
  document.dispatchEvent(new Event('selectionchange'));
}

function typeTemplate(fixture: { nativeElement: HTMLElement }, value: string): void {
  const input = (fixture.nativeElement as HTMLElement).querySelector<HTMLInputElement>('input.input')!;
  input.value = value;
  input.dispatchEvent(new Event('input'));
}

const TEMPLATE = '{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}';

describe('FormatWizard', () => {
  const ref = { close: vi.fn(), closed: Promise.resolve(undefined) };

  async function mount(backend = tinyBackend(), path = '/logs/system/export.reg') {
    ref.close.mockClear();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), { provide: DIALOG_REF, useValue: ref }] });
    const fixture = TestBed.createComponent(FormatWizard as Type<FormatWizard>);
    fixture.componentRef.setInput('path', path);
    fixture.componentRef.setInput('name', 'export.reg');
    await settle(fixture);
    return { fixture, backend, store: TestBed.inject(WorkspaceStore) };
  }

  /** Lets the debounced engine preview run. */
  async function preview(fixture: Awaited<ReturnType<typeof mount>>['fixture']) {
    await new Promise((r) => setTimeout(r, 300));
    await settle(fixture);
  }

  async function mark(fixture: Awaited<ReturnType<typeof mount>>['fixture'], needle: string, label: string) {
    const span = all(fixture, '[data-testid=sample-line] span').find((s) => s.textContent?.includes(needle))!;
    selectText(span, needle);
    await settle(fixture);
    buttonByText(fixture, new RegExp('^\\s*' + label)).click();
    await settle(fixture);
  }

  it('loads the sample with read_file_head and has no client-side parser', async () => {
    const backend = tinyBackend();
    const head = vi.spyOn(backend, 'readFileHead');
    const { fixture } = await mount(backend);
    expect(head).toHaveBeenCalledWith('/logs/system/export.reg', 32768);
    expect(el(fixture, '[data-testid=sample-line]').textContent).toContain('[INF]');
    expect(all(fixture, 'select option')).toHaveLength(5);
  });

  it('generates a regex from marks, asks the engine to preview it and saves it first in parse.regexes', async () => {
    const backend = tinyBackend();
    const requests: ParsePreviewRequest[] = [];
    const original = backend.previewParse.bind(backend);
    vi.spyOn(backend, 'previewParse').mockImplementation((r) => (requests.push(r), original(r)));
    const { fixture, store } = await mount(backend);
    expect(el(fixture, '[data-testid=format-regex]').textContent).toContain('Mark a timestamp');
    await mark(fixture, '2026-10-14 10:41:02.114 +00:00', 'Timestamp');
    await mark(fixture, 'INF', 'Level');
    await preview(fixture);
    expect(requests).toHaveLength(0);
    await mark(fixture, 'Order 1042 accepted for customer 77', 'Message');
    expect(el(fixture, '[data-testid=format-regex]').textContent).toContain('(?P<timestamp>');
    await preview(fixture);
    expect(requests).toHaveLength(1);
    expect(requests[0]).toMatchObject({ fileName: 'export.reg', limit: 50 });
    expect(requests[0].toml).toMatch(/regexes = \[\s*"\^\(\?P<timestamp>/);
    expect(requests[0].sample).toContain('Payment capture failed');
    expect(el(fixture, '[data-testid=format-summary]').textContent).toContain('4 events parsed');
    expect(all(fixture, '[role=row]')).toHaveLength(5);
    expect(text(fixture)).toContain('Payment capture failed');
    buttonByText(fixture, 'Save and re-import').click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(true));
    expect((await backend.getProfile()).toml).toMatch(/regexes = \[\s*"\^\(\?P<timestamp>/);
    expect(store.notice()).toMatch(/Saved the format/);
  });

  it('supports Serilog templates, other sample lines and clearing marks', async () => {
    const { fixture, backend } = await mount();
    const select = el<HTMLSelectElement>(fixture, 'select');
    select.value = '1';
    select.dispatchEvent(new Event('change'));
    await settle(fixture);
    expect(el(fixture, '[data-testid=sample-line]').textContent).toContain('Payment gateway slow');
    await mark(fixture, '2026-10-14 10:41:02.980 +00:00', 'Timestamp');
    buttonByText(fixture, 'Clear marks').click();
    await settle(fixture);
    expect(all(fixture, '.hl')).toHaveLength(0);

    buttonByText(fixture, 'Serilog output template').click();
    await settle(fixture);
    expect(all(fixture, '[data-testid=format-regex]')).toHaveLength(0);
    typeTemplate(fixture, TEMPLATE);
    await preview(fixture);
    expect(el(fixture, '[data-testid=format-summary]').textContent).toContain('4 events parsed');
    buttonByText(fixture, 'Save and re-import').click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(true));
    expect((await backend.getProfile()).toml).toContain('{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz}');
  });

  it('explains when the file cannot be read, and retries', async () => {
    const backend = tinyBackend();
    const head = vi.spyOn(backend, 'readFileHead').mockRejectedValueOnce('only files in the workspace can be previewed');
    const { fixture } = await mount(backend);
    expect(el(fixture, '[role=alert]').textContent).toContain('only files in the workspace');
    buttonByText(fixture, 'Serilog output template').click();
    await settle(fixture);
    typeTemplate(fixture, '{Timestamp} {Message}');
    await preview(fixture);
    expect(buttonByText(fixture, 'Save and re-import').disabled).toBe(true);
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(head).toHaveBeenCalledTimes(2);
    expect(all(fixture, '[role=alert]')).toHaveLength(0);
    buttonByText(fixture, 'Mark on a sample line').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=sample-line]').textContent).toContain('[INF]');
  });

  it('refuses a path outside the workspace', async () => {
    const { fixture } = await mount(tinyBackend(), '/etc/passwd');
    expect(el(fixture, '[role=alert]').textContent).toContain('only files in the workspace');
  });

  it('shows the engine error, keeps save disabled for it and reports save errors', async () => {
    const backend = tinyBackend();
    const { fixture } = await mount(backend);
    buttonByText(fixture, 'Serilog output template').click();
    await settle(fixture);
    vi.spyOn(backend, 'previewParse').mockResolvedValueOnce({ importer: null, records: [], total: 0, skipped: 5, dropped: 0, error: null });
    typeTemplate(fixture, '{Timestamp} {Message}');
    await preview(fixture);
    expect(el(fixture, '[data-testid=format-summary]').textContent).toContain('0 events parsed');
    expect(buttonByText(fixture, 'Save and re-import').disabled).toBe(true);

    typeTemplate(fixture, TEMPLATE);
    await preview(fixture);
    vi.spyOn(backend, 'saveProfile').mockRejectedValue(new Error('cannot save'));
    buttonByText(fixture, 'Save and re-import').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=format-summary]').textContent).toContain('4 events');
    expect(all(fixture, '[role=alert]').map((a) => a.textContent).join()).toContain('cannot save');

    vi.spyOn(backend, 'previewParse').mockResolvedValueOnce({ importer: null, records: [], total: 0, skipped: 0, dropped: 0, error: 'bad template' });
    typeTemplate(fixture, TEMPLATE + ' ');
    await preview(fixture);
    expect(el(fixture, '[data-testid=format-summary]').textContent).toContain('bad template');
    expect(buttonByText(fixture, 'Save and re-import').disabled).toBe(true);

    vi.spyOn(backend, 'previewParse').mockRejectedValueOnce('engine unavailable');
    typeTemplate(fixture, TEMPLATE + '  ');
    await preview(fixture);
    expect(el(fixture, '[data-testid=format-summary]').textContent).toContain('engine unavailable');
    typeTemplate(fixture, '');
    await settle(fixture);
    expect(all(fixture, '[data-testid=format-summary]')).toHaveLength(0);
  });

  it('shows rows without a time as a dash', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'previewParse').mockResolvedValue({
      importer: 'text',
      records: [{ line: 1, ts: null, level: 'Information', message: 'no time', exception: null, fields: {}, markers: [] }],
      total: 1,
      skipped: 0,
      dropped: 0,
      error: null,
    });
    const { fixture } = await mount(backend);
    buttonByText(fixture, 'Serilog output template').click();
    await settle(fixture);
    typeTemplate(fixture, TEMPLATE);
    await preview(fixture);
    expect(all(fixture, '[role=row]')[1].textContent).toContain('–');
  });

  it('ignores a mark without a selection and replaces overlapping marks', async () => {
    const { fixture } = await mount();
    const inst = fixture.componentInstance as unknown as { mark(p: 'level'): void; marks: () => Record<string, unknown>; pending: { set(v: unknown): void } };
    inst.mark('level');
    expect(inst.marks()).toEqual({});
    inst.pending.set({ start: 0, end: 10 });
    inst.mark('level');
    inst.pending.set({ start: 5, end: 12 });
    inst.mark('level');
    expect(Object.keys(inst.marks())).toEqual(['level']);
    inst.pending.set({ start: 20, end: 22 });
    (inst as unknown as { mark(p: string): void }).mark('timestamp');
    expect(Object.keys(inst.marks()).sort()).toEqual(['level', 'timestamp']);
  });

  it('does nothing on save without anything to save', async () => {
    const { fixture } = await mount();
    await (fixture.componentInstance as unknown as { save(): Promise<void> }).save();
    expect(ref.close).not.toHaveBeenCalled();
  });
});
