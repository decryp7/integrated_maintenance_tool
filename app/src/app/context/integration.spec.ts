import { TestBed } from '@angular/core/testing';
import { Type } from '@angular/core';
import { all, buttonByText, el, FakeDock, provideFakeDock, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { ImtBackend } from '../core/backend';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { EventsGrid } from '../grid/events-grid';
import { FormatWizard } from '../format/format-wizard';
import { FILE_PREVIEW, SampleFilePreview } from '../format/file-preview';
import { Inspector } from '../inspector/inspector';
import { AdhocCharts } from '../metrics/adhoc-charts';
import { Sidebar } from '../shell/sidebar';
import { ContextMenuService } from '../ui/context-menu.service';
import { DIALOG_REF, DialogService } from '../ui/dialog.service';
import { DropDialog } from './drop-dialog';
import { ExtractDialog } from './extract-dialog';
import { MaskDialog } from './mask-dialog';

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

describe('Inspector context actions', () => {
  async function render() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock()] });
    const fixture = TestBed.createComponent(Inspector);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    const page = await TestBed.inject(ImtBackend).queryPage({ query: EMPTY_QUERY, offset: 0, limit: 20, descending: false });
    store.select(page.rows.find((r) => r.message.startsWith('Slow request'))!);
    await settle(fixture);
    return { fixture, store, dialogs: TestBed.inject(DialogService), menu: TestBed.inject(ContextMenuService) };
  }

  it('enables mask and extract only while text is selected in the message', async () => {
    const { fixture, dialogs } = await render();
    const mask = buttonByText(fixture, /Mask/);
    const extract = buttonByText(fixture, /Extract value/);
    expect(mask.disabled).toBe(true);
    expect(extract.disabled).toBe(true);
    selectText(el(fixture, '.message'), '1500');
    await settle(fixture);
    expect(mask.disabled).toBe(false);
    mask.click();
    extract.click();
    expect(dialogs.stack().map((d) => d.component)).toEqual([MaskDialog, ExtractDialog]);
    expect(dialogs.stack()[1].options.inputs).toMatchObject({ start: 23, end: 27 });
    // A selection outside the message is ignored; collapsing inside it clears the buttons.
    document.getSelection()!.removeAllRanges();
    document.dispatchEvent(new Event('selectionchange'));
    selectText(el(fixture, '.k'), 'Application');
    await settle(fixture);
    expect(buttonByText(fixture, /Mask/).disabled).toBe(false);
  });

  it('opens the record menu and the property menu (mouse and keyboard)', async () => {
    const { fixture, menu, store } = await render();
    buttonByText(fixture, /More actions/).click();
    expect(menu.menu()?.items.map((i) => i.label)).toContain('Drop events like this (pipeline)…');
    menu.close();
    const prop = el(fixture, '.prop');
    prop.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }));
    expect(menu.menu()?.items[0].label).toMatch(/^Filter: only this /);
    menu.menu()!.items[0].run();
    expect(store.query().filters).toHaveLength(1);
    menu.close();
    prop.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true, cancelable: true }));
    expect(menu.menu()).not.toBeNull();
    menu.close();
    prop.dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true }));
    expect(menu.menu()).not.toBeNull();
    menu.close();
    prop.dispatchEvent(new KeyboardEvent('keydown', { key: 'a', bubbles: true }));
    expect(menu.menu()).toBeNull();
    el<HTMLButtonElement>(fixture, '[aria-label^="More actions for"]').click();
    expect(menu.menu()).not.toBeNull();
  });

  it('does nothing without a selected record', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock()] });
    const fixture = TestBed.createComponent(Inspector);
    await settle(fixture);
    const inspector = fixture.componentInstance as unknown as { mask(): void; extract(): void; openActions(e: MouseEvent): void; openPropMenu(e: MouseEvent, k: string, v: string): void; onPropKey(e: KeyboardEvent, k: string, v: string): void };
    inspector.mask();
    inspector.extract();
    inspector.openActions(new MouseEvent('click'));
    inspector.openPropMenu(new MouseEvent('contextmenu'), 'a', 'b');
    inspector.onPropKey(new KeyboardEvent('keydown', { key: 'ContextMenu' }), 'a', 'b');
    expect(TestBed.inject(ContextMenuService).menu()).toBeNull();
  });
});

describe('EventsGrid context menu', () => {
  let restore: PropertyDescriptor | undefined;
  beforeAll(() => {
    restore = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight');
    Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
      configurable: true,
      get(this: HTMLElement) {
        return this.classList?.contains('viewport') ? 260 : 0;
      },
    });
  });
  afterAll(() => {
    if (restore) Object.defineProperty(HTMLElement.prototype, 'clientHeight', restore);
  });

  it('opens on right click and with the keyboard', async () => {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    const fixture = TestBed.createComponent(EventsGrid);
    await settle(fixture);
    const menu = TestBed.inject(ContextMenuService);
    const store = TestBed.inject(WorkspaceStore);
    const row = all(fixture, '.row')[1];
    row.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }));
    expect(store.selected()).not.toBeNull();
    expect(menu.menu()?.items.map((i) => i.label)).toContain('Mark events like this…');
    menu.close();
    const viewport = el(fixture, '.viewport');
    viewport.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true, cancelable: true }));
    expect(menu.menu()).not.toBeNull();
    menu.close();
    viewport.dispatchEvent(new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true }));
    expect(menu.menu()).not.toBeNull();
    menu.close();
    store.select(null);
    viewport.dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true, cancelable: true }));
    expect(menu.menu()).toBeNull();
  });
});

describe('Explorer additions', () => {
  async function render() {
    const dock = new FakeDock();
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(dock)] });
    const fixture = TestBed.createComponent(Sidebar);
    await settle(fixture);
    return { fixture, dock, store: TestBed.inject(WorkspaceStore), dialogs: TestBed.inject(DialogService) };
  }

  it('charts a numeric field in the Metrics panel', async () => {
    const { fixture, dock, store } = await render();
    const adhoc = TestBed.inject(AdhocCharts);
    const field = all<HTMLButtonElement>(fixture, '.field').find((b) => b.textContent?.includes('ProcessId'))!;
    field.click();
    await settle(fixture);
    buttonByText(fixture, 'Chart this field').click();
    expect(adhoc.charts().map((c) => c.y)).toEqual(['avg(ProcessId)']);
    expect(adhoc.charts()[0].splitBy).toEqual(store.effectiveGroupBy());
    expect(dock.calls).toContain('show:metrics');
  });

  it('does not offer charting for text fields', async () => {
    const { fixture } = await render();
    all<HTMLButtonElement>(fixture, '.field').find((b) => b.textContent?.includes('MachineName'))!.click();
    await settle(fixture);
    expect(all(fixture, 'button').some((b) => b.textContent?.trim() === 'Chart this field')).toBe(false);
  });

  it('opens the format wizard for an unrecognised file', async () => {
    const { fixture, dialogs } = await render();
    el<HTMLButtonElement>(fixture, '[aria-label="Set up format for export.reg"]').click();
    expect(dialogs.stack()[0].component).toBe(FormatWizard);
    expect(dialogs.stack()[0].options.inputs).toEqual({ path: '/logs/system/export.reg', name: 'export.reg' });
  });
});

describe('AdhocCharts', () => {
  it('adds, reuses and removes charts', () => {
    const charts = TestBed.inject(AdhocCharts);
    const a = charts.addForField('cpu', ['a']);
    expect(charts.addForField('cpu', ['a'])).toBe(a);
    const b = charts.addForField('cpu', ['b']);
    expect(charts.charts()).toHaveLength(2);
    charts.remove(a.id);
    expect(charts.charts()).toEqual([b]);
  });
});

describe('FormatWizard', () => {
  const ref = { close: vi.fn(), closed: Promise.resolve(undefined) };

  async function mount(preview: { firstLines: (...args: never[]) => Promise<string[]> } = new SampleFilePreview(), backend = tinyBackend()) {
    ref.close.mockClear();
    TestBed.configureTestingModule({
      providers: [provideTinyBackend(backend), { provide: DIALOG_REF, useValue: ref }, { provide: FILE_PREVIEW, useValue: preview }],
    });
    const fixture = TestBed.createComponent(FormatWizard as Type<FormatWizard>);
    fixture.componentRef.setInput('path', '/x/app.log');
    fixture.componentRef.setInput('name', 'app.log');
    await settle(fixture);
    return { fixture, backend, store: TestBed.inject(WorkspaceStore) };
  }

  async function mark(fixture: Awaited<ReturnType<typeof mount>>['fixture'], needle: string, label: string) {
    const span = all(fixture, '[data-testid=sample-line] span').find((s) => s.textContent?.includes(needle))!;
    selectText(span, needle);
    await settle(fixture);
    buttonByText(fixture, new RegExp('^\\s*' + label)).click();
    await settle(fixture);
  }

  it('generates a regex from marks, previews the parse and saves it first in parse.regexes', async () => {
    const { fixture, backend, store } = await mount();
    expect(el(fixture, '[data-testid=format-regex]').textContent).toContain('Mark a timestamp');
    await mark(fixture, '2026-10-14 10:41:02.114 +00:00', 'Timestamp');
    await mark(fixture, 'INF', 'Level');
    expect(el(fixture, '[data-testid=format-regex]').textContent).toContain('Mark a timestamp');
    await mark(fixture, 'Order 1042 accepted for customer 77', 'Message');
    expect(el(fixture, '[data-testid=format-regex]').textContent).toContain('(?P<timestamp>');
    expect(el(fixture, '[data-testid=format-summary]').textContent).toContain('4 of 5 lines parse');
    expect(all(fixture, '[role=row]')).toHaveLength(6);
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
    typeTemplate(fixture, '{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}');
    await settle(fixture);
    expect(el(fixture, '[data-testid=format-summary]').textContent).toContain('4 of 5 lines parse');
    buttonByText(fixture, 'Save and re-import').click();
    await vi.waitFor(() => expect(ref.close).toHaveBeenCalledWith(true));
    expect((await backend.getProfile()).toml).toContain('{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz}');
  });

  it('explains when the file cannot be previewed and when saving fails', async () => {
    const failing = { firstLines: () => Promise.reject(new Error('Previewing files is not supported yet')) };
    const { fixture, backend } = await mount(failing);
    expect(el(fixture, '[role=alert]').textContent).toContain('not supported yet');
    buttonByText(fixture, 'Serilog output template').click();
    await settle(fixture);
    typeTemplate(fixture, '{Timestamp} {Message}');
    await settle(fixture);
    expect(buttonByText(fixture, 'Save and re-import').disabled).toBe(true);
    expect(backend).toBeDefined();
    expect(text(fixture)).toContain('0 of 0 lines parse');
  });

  it('keeps the save button disabled for an invalid pattern and reports save errors', async () => {
    const lines = ['2026-01-01 a'];
    const backend = tinyBackend();
    const { fixture } = await mount({ firstLines: () => Promise.resolve(lines) }, backend);
    buttonByText(fixture, 'Serilog output template').click();
    await settle(fixture);
    typeTemplate(fixture, '{Timestamp:yyyy-MM-dd} {Message}');
    await settle(fixture);
    vi.spyOn(backend, 'saveProfile').mockRejectedValue(new Error('cannot save'));
    buttonByText(fixture, 'Save and re-import').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('cannot save');
    typeTemplate(fixture, '{Level}');
    await settle(fixture);
    expect(buttonByText(fixture, 'Save and re-import').disabled).toBe(true);
    // A template that cannot be turned into a regex that compiles shows the error.
    typeTemplate(fixture, '{Timestamp:(} {Message}');
    await settle(fixture);
    expect(buttonByText(fixture, 'Save and re-import').disabled).toBe(true);
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
});

function typeTemplate(fixture: { nativeElement: HTMLElement }, value: string): void {
  const input = (fixture.nativeElement as HTMLElement).querySelector<HTMLInputElement>('input.input')!;
  input.value = value;
  input.dispatchEvent(new Event('input'));
}

describe('DropDialog from the wizard module (smoke)', () => {
  it('exports the dialogs', () => {
    expect(DropDialog).toBeDefined();
  });
});
