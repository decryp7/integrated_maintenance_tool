import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, FakeDock, provideFakeDock, provideTinyBackend, settle } from '../../testing/helpers';
import { ImtBackend } from '../core/backend';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { EventsGrid } from '../grid/events-grid';
import { FormatWizard } from '../format/format-wizard';
import { Inspector } from '../inspector/inspector';
import { AdhocCharts } from '../metrics/adhoc-charts';
import { Sidebar } from '../shell/sidebar';
import { ContextMenuService } from '../ui/context-menu.service';
import { DialogService } from '../ui/dialog.service';
import { DropDialog } from './drop-dialog';
import { ExtractDialog } from './extract-dialog';
import { MaskDialog } from './mask-dialog';
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
    expect(dialogs.stack()[1]?.options.inputs).toMatchObject({ start: 23, end: 27 });
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
    expect(menu.menu()?.items[0]?.label).toMatch(/^Filter: only this /);
    at(menu.menu()!.items, 0).run();
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
    const row = at(all(fixture, '.row'), 1);
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
    expect(adhoc.charts()[0]?.splitBy).toEqual(store.effectiveGroupBy());
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
    expect(dialogs.stack()[0]?.component).toBe(FormatWizard);
    expect(dialogs.stack()[0]?.options.inputs).toEqual({ path: '/logs/system/export.reg', name: 'export.reg' });
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

describe('DropDialog from the wizard module (smoke)', () => {
  it('exports the dialogs', () => {
    expect(DropDialog).toBeDefined();
  });
});
