import { Type } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { ParsePreviewRequest } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { MockParsersBackend } from '../settings/parsers/mock-parsers-backend';
import { ParsersBackend } from '../settings/parsers/parsers-backend';
import { DIALOG_REF } from '../ui/dialog.service';
import { FormatWizard } from './format-wizard';
import { at } from '../../testing/must';

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

  async function mount(backend = tinyBackend(), path = '/logs/system/export.reg', timestampFormat?: string | null) {
    ref.close.mockClear();
    const parsers = new MockParsersBackend(backend);
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), { provide: DIALOG_REF, useValue: ref }, { provide: ParsersBackend, useValue: parsers }] });
    const fixture = TestBed.createComponent(FormatWizard as Type<FormatWizard>);
    fixture.componentRef.setInput('path', path);
    fixture.componentRef.setInput('name', path.split('/').pop());
    if (timestampFormat !== undefined) fixture.componentRef.setInput('timestampFormat', timestampFormat);
    await settle(fixture);
    return { fixture, backend, parsers, store: TestBed.inject(WorkspaceStore) };
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

  it('generates a regex from marks, previews the parser the engine drafts and saves it as a new parser', async () => {
    const backend = tinyBackend();
    const requests: ParsePreviewRequest[] = [];
    const original = backend.previewParse.bind(backend);
    vi.spyOn(backend, 'previewParse').mockImplementation((r) => (requests.push(r), original(r)));
    const { fixture, store, parsers } = await mount(backend);
    const draft = vi.spyOn(parsers, 'draft');
    const reread = vi.spyOn(parsers, 'reread');
    expect(el(fixture, '[data-testid=format-regex]').textContent).toContain('Mark a timestamp');
    expect(text(fixture)).toContain('IMT did not recognise');
    expect(text(fixture)).not.toContain('template…');
    await mark(fixture, '2026-10-14 10:41:02.114 +00:00', 'Timestamp');
    await mark(fixture, 'INF', 'Level');
    await preview(fixture);
    expect(requests).toHaveLength(0);
    await mark(fixture, 'Order 1042 accepted for customer 77', 'Message');
    expect(el(fixture, '[data-testid=format-regex]').textContent).toContain('(?P<timestamp>');
    await preview(fixture);
    expect(requests).toHaveLength(1);
    expect(requests[0]).toMatchObject({ fileName: 'export.reg', limit: 50 });
    expect(draft).toHaveBeenLastCalledWith(expect.objectContaining({ fileName: 'export.reg', base: null, template: null, timestampFormat: null }));
    expect(at(requests, 0).toml).toContain('schema = 1');
    expect(at(requests, 0).toml).toMatch(/regexes = \[\s*"\^\(\?P<timestamp>/);
    expect(at(requests, 0).sample).toContain('Payment capture failed');
    expect(el(fixture, '[data-testid=format-summary]').textContent).toContain('4 events parsed');
    expect(all(fixture, '[role=row]')).toHaveLength(5);
    expect(text(fixture)).toContain('Payment capture failed');
    buttonByText(fixture, 'Save parser and re-read').click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(true));
    expect((await parsers.get('export.reg')).toml).toMatch(/regexes = \[\s*"\^\(\?P<timestamp>/);
    expect(reread).toHaveBeenCalled();
    expect(store.notice()).toBe('Saved the parser export.reg; the files are read again.');
  });

  it('supports Serilog templates, other sample lines and clearing marks', async () => {
    const { fixture, parsers } = await mount();
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
    buttonByText(fixture, 'Save parser and re-read').click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(true));
    expect((await parsers.get('export.reg')).toml).toContain('{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz}');
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
    expect(buttonByText(fixture, 'Save parser and re-read').disabled).toBe(true);
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
    const { fixture, parsers } = await mount(backend);
    buttonByText(fixture, 'Serilog output template').click();
    await settle(fixture);
    vi.spyOn(backend, 'previewParse').mockResolvedValueOnce({ importer: null, records: [], total: 0, skipped: 5, dropped: 0, error: null });
    typeTemplate(fixture, '{Timestamp} {Message}');
    await preview(fixture);
    expect(el(fixture, '[data-testid=format-summary]').textContent).toContain('0 events parsed');
    expect(buttonByText(fixture, 'Save parser and re-read').disabled).toBe(true);

    typeTemplate(fixture, TEMPLATE);
    await preview(fixture);
    vi.spyOn(parsers, 'save').mockRejectedValue(new Error('cannot save'));
    buttonByText(fixture, 'Save parser and re-read').click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=format-summary]').textContent).toContain('4 events');
    expect(all(fixture, '[role=alert]').map((a) => a.textContent).join()).toContain('cannot save');

    vi.spyOn(backend, 'previewParse').mockResolvedValueOnce({ importer: null, records: [], total: 0, skipped: 0, dropped: 0, error: 'bad template' });
    typeTemplate(fixture, TEMPLATE + ' ');
    await preview(fixture);
    expect(el(fixture, '[data-testid=format-summary]').textContent).toContain('bad template');
    expect(buttonByText(fixture, 'Save parser and re-read').disabled).toBe(true);

    vi.spyOn(parsers, 'draft').mockRejectedValueOnce('engine unavailable');
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
    expect(at(all(fixture, '[role=row]'), 1).textContent).toContain('–');
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

  it('offers the timestamp format only when opened for unreadable timestamps', async () => {
    const { fixture } = await mount();
    expect(all(fixture, 'button').some((b) => b.textContent?.includes('Timestamp format'))).toBe(false);
    expect(all(fixture, '[data-testid=timestamp-format]')).toHaveLength(0);
  });

  it('starts on the proposed timestamp format, adds it to the parser that read the file and saves an override', async () => {
    const backend = tinyBackend();
    const requests: ParsePreviewRequest[] = [];
    const original = backend.previewParse.bind(backend);
    vi.spyOn(backend, 'previewParse').mockImplementation((r) => (requests.push(r), original(r)));
    const { fixture, parsers } = await mount(backend, '/logs/web02/app.log', '%a %b %d %H:%M:%S %Y');
    const save = vi.spyOn(parsers, 'save');
    expect(text(fixture)).toContain('but not their timestamps');
    const input = el(fixture, '[data-testid=timestamp-format]') as HTMLInputElement;
    expect(input.value).toBe('%a %b %d %H:%M:%S %Y');
    expect(text(fixture)).toContain('IMT checked this format');
    await preview(fixture);
    expect(at(requests, 0).toml).toContain('timestamp_formats = [ "%a %b %d %H:%M:%S %Y" ]');
    expect(at(requests, 0).toml).toContain('id = "serilog-text"');
    // An edited format is no longer the checked one; an empty one has nothing to preview.
    input.value = '%Y';
    input.dispatchEvent(new Event('input'));
    await settle(fixture);
    expect(text(fixture)).not.toContain('IMT checked this format');
    input.value = '  ';
    input.dispatchEvent(new Event('input'));
    await settle(fixture);
    expect(all(fixture, '[data-testid=format-summary]')).toHaveLength(0);
    input.value = '%a %b %d %H:%M:%S %Y';
    input.dispatchEvent(new Event('input'));
    await preview(fixture);
    buttonByText(fixture, 'Save parser and re-read').click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(true));
    expect(save).toHaveBeenCalledWith(expect.stringContaining('%a %b %d %H:%M:%S %Y'), { previousId: 'serilog-text', overwrite: true });
    const saved = await parsers.get('serilog-text');
    expect(saved.origin).toBe('override');
    expect(saved.toml).toContain('timestamp_formats = [ "%a %b %d %H:%M:%S %Y" ]');
    // The other modes stay available.
    expect(all(fixture, 'button').some((b) => b.textContent?.includes('Mark on a sample line'))).toBe(true);
  });

  it('starts on an empty timestamp format when the engine had none to propose', async () => {
    const { fixture } = await mount(tinyBackend(), '/logs/system/export.reg', null);
    expect((el(fixture, '[data-testid=timestamp-format]') as HTMLInputElement).value).toBe('');
    expect(text(fixture)).not.toContain('IMT checked this format');
  });

  it('does nothing on save without anything to save', async () => {
    const { fixture } = await mount();
    await (fixture.componentInstance as unknown as { save(): Promise<void> }).save();
    expect(ref.close).not.toHaveBeenCalled();
  });
});
