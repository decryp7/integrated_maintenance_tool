import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { Page, PageRequest, RecordView } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { MockBackend } from '../core/mock/mock-backend';
import { tinyDataset } from '../../testing/helpers';
import { LiveTail } from '../workspace/live-tail';
import { WorkspaceStore } from '../core/workspace.store';
import { SettingsStore } from '../settings/settings.store';
import { EventsGrid } from './events-grid';
import { RecordActions } from '../context/record-actions';
import { HighlightRules } from '../projects/bookmarks/highlight';

const VIEWPORT_H = 260;

function fakeRow(i: number): RecordView {
  return { id: i, ts: 1_700_000_000_000 + i, level: 'Information', message: `row ${i}`, template: null, exception: null, fields: { n: i }, source: 0, sourceName: 's.log', line: i + 1, group: 'g', markers: [] };
}

/** Backend with a huge synthetic result to exercise paging and scroll compression. */
class HugeBackend extends MockBackend {
  requests: PageRequest[] = [];
  constructor(private readonly total: number) {
    super({ latency: false, dataset: () => ({ sources: [], unrecognized: [], records: [] }) });
  }
  override queryPage(req: PageRequest): Promise<Page> {
    this.requests.push(req);
    if (req.anchor != null) return Promise.resolve({ total: this.total, offset: req.anchor, rows: [fakeRow(req.anchor)], anchorIndex: req.anchor });
    const rows = Array.from({ length: Math.max(0, Math.min(req.limit, this.total - req.offset)) }, (_, k) => fakeRow(req.offset + k));
    return Promise.resolve({ total: this.total, offset: req.offset, rows, anchorIndex: null });
  }
}

describe('EventsGrid', () => {
  let restore: PropertyDescriptor | undefined;
  beforeAll(() => {
    restore = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight');
    Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
      configurable: true,
      get(this: HTMLElement) {
        return this.classList?.contains('viewport') ? VIEWPORT_H : 0;
      },
    });
  });
  afterAll(() => {
    if (restore) Object.defineProperty(HTMLElement.prototype, 'clientHeight', restore);
  });
  beforeEach(() => localStorage.clear());

  async function render(backend: ImtBackend = tinyBackend()) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(EventsGrid);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    return { fixture, store };
  }

  const rows = (f: Awaited<ReturnType<typeof render>>['fixture']) => all(f, '.row:not(.placeholder)');

  it('renders the first page with totals', async () => {
    const { fixture, store } = await render();
    expect(rows(fixture)).toHaveLength(11);
    expect(el(fixture, '.total').textContent).toBe('11');
    expect(text(fixture)).toContain('of 11');
    expect(store.matchTotal()).toBe(11);
    expect(rows(fixture)[0].textContent).toContain('Application starting');
    expect(all(fixture, '.row imt-marker-icon').length).toBeGreaterThan(0);
  });

  it('follows the density and date format settings', async () => {
    const { fixture } = await render();
    const settings = TestBed.inject(SettingsStore);
    expect(fixture.nativeElement.style.getPropertyValue('--row-h')).toBe('26px');
    expect(rows(fixture)[0].querySelector('.time')?.textContent).toMatch(/^\d+ \w{3} /);
    settings.setDensity('compact');
    settings.setDateFormat('iso');
    await settle(fixture);
    expect(fixture.nativeElement.style.getPropertyValue('--row-h')).toBe('22px');
    expect(rows(fixture)[0].querySelector('.time')?.textContent).toMatch(/^\d{4}-\d{2}-\d{2} /);
  });

  it('selects rows by click and keyboard', async () => {
    const { fixture, store } = await render();
    rows(fixture)[2].click();
    await settle(fixture);
    expect(store.selected()?.id).toBe(2);
    expect(rows(fixture)[2].classList.contains('sel')).toBe(true);
    const vp = el(fixture, '.viewport');
    const key = (k: string) => vp.dispatchEvent(new KeyboardEvent('keydown', { key: k, cancelable: true }));
    key('ArrowDown');
    expect(store.selected()?.id).toBe(3);
    key('ArrowUp');
    key('ArrowUp');
    expect(store.selected()?.id).toBe(1);
    key('End');
    expect(store.selected()?.id).toBe(10);
    key('Home');
    expect(store.selected()?.id).toBe(0);
    key('PageDown');
    expect(store.selected()!.id).toBeGreaterThan(0);
    key('PageUp');
    expect(store.selected()?.id).toBe(0);
    key('x');
    expect(store.selected()?.id).toBe(0);
  });

  describe('multi-select', () => {
    const click = (row: HTMLElement, init: MouseEventInit = {}) => row.dispatchEvent(new MouseEvent('click', { bubbles: true, ...init }));
    const selectedIndexes = (f: Awaited<ReturnType<typeof render>>['fixture']) =>
      rows(f)
        .map((r, i) => (r.getAttribute('aria-selected') === 'true' ? i : -1))
        .filter((i) => i >= 0);

    it('is announced as a multi-selectable grid', async () => {
      const { fixture } = await render();
      expect(el(fixture, '[role=grid]').getAttribute('aria-multiselectable')).toBe('true');
      expect(rows(fixture).every((r) => r.getAttribute('aria-selected') === 'false')).toBe(true);
    });

    it('shift+click selects a range, ctrl/cmd+click toggles, a plain click selects one row', async () => {
      const { fixture, store } = await render();
      click(rows(fixture)[2]);
      await settle(fixture);
      click(rows(fixture)[5], { shiftKey: true });
      await settle(fixture);
      expect(selectedIndexes(fixture)).toEqual([2, 3, 4, 5]);
      expect(store.selected()?.id).toBe(5);
      click(rows(fixture)[0], { shiftKey: true });
      await settle(fixture);
      expect(selectedIndexes(fixture)).toEqual([0, 1, 2]);
      click(rows(fixture)[1], { ctrlKey: true });
      await settle(fixture);
      expect(selectedIndexes(fixture)).toEqual([0, 2]);
      expect(store.selected()?.id).toBe(1);
      click(rows(fixture)[8], { metaKey: true });
      await settle(fixture);
      expect(selectedIndexes(fixture)).toEqual([0, 2, 8]);
      click(rows(fixture)[6]);
      await settle(fixture);
      expect(selectedIndexes(fixture)).toEqual([6]);
      expect(rows(fixture)[6].classList.contains('sel')).toBe(true);
    });

    it('prevents text selection only for modified clicks', async () => {
      const { fixture } = await render();
      const down = (init: MouseEventInit) => {
        const e = new MouseEvent('mousedown', { bubbles: true, cancelable: true, ...init });
        rows(fixture)[1].dispatchEvent(e);
        return e.defaultPrevented;
      };
      expect(down({})).toBe(false);
      expect(down({ shiftKey: true })).toBe(true);
      expect(down({ ctrlKey: true })).toBe(true);
      expect(down({ metaKey: true })).toBe(true);
    });

    it('shift+arrow extends the selection from the anchor', async () => {
      const { fixture, store } = await render();
      const vp = el(fixture, '.viewport');
      const key = (k: string, shiftKey = false) => vp.dispatchEvent(new KeyboardEvent('keydown', { key: k, shiftKey, cancelable: true }));
      click(rows(fixture)[3]);
      key('ArrowDown', true);
      key('ArrowDown', true);
      await settle(fixture);
      expect(selectedIndexes(fixture)).toEqual([3, 4, 5]);
      expect(store.selected()?.id).toBe(5);
      key('ArrowUp', true);
      key('ArrowUp', true);
      key('ArrowUp', true);
      await settle(fixture);
      expect(selectedIndexes(fixture)).toEqual([2, 3]);
      key('ArrowDown');
      await settle(fixture);
      expect(selectedIndexes(fixture)).toEqual([3]);
    });

    it('the context menu acts on all selected rows, or on the clicked row outside the selection', async () => {
      const { fixture, store } = await render();
      const actions = TestBed.inject(RecordActions);
      const open = vi.spyOn(actions, 'openFor').mockImplementation(() => undefined);
      const openAt = vi.spyOn(actions, 'openAtElement').mockImplementation(() => undefined);
      click(rows(fixture)[1]);
      click(rows(fixture)[3], { shiftKey: true });
      await settle(fixture);
      rows(fixture)[2].dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }));
      await settle(fixture);
      expect(open.mock.calls[0][1].id).toBe(2);
      expect(open.mock.calls[0][2]?.records?.map((r) => r.id)).toEqual([1, 2, 3]);
      expect(selectedIndexes(fixture)).toEqual([1, 2, 3]);
      expect(store.selected()?.id).toBe(2);

      el(fixture, '.viewport').dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', cancelable: true }));
      expect(openAt.mock.calls[0][1].id).toBe(2);
      expect(openAt.mock.calls[0][2]?.records?.map((r) => r.id)).toEqual([1, 2, 3]);
      expect(openAt.mock.calls[0][0]).toBe(rows(fixture)[2]);

      rows(fixture)[7].dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }));
      await settle(fixture);
      expect(open.mock.calls[1][2]?.records?.map((r) => r.id)).toEqual([7]);
      expect(selectedIndexes(fixture)).toEqual([7]);
    });

    it('collapses to the record selected elsewhere (timeline, bookmark) and clears on new data', async () => {
      const { fixture, store } = await render();
      click(rows(fixture)[1]);
      click(rows(fixture)[4], { shiftKey: true });
      await settle(fixture);
      store.jumpTo(8);
      await settle(fixture);
      expect(selectedIndexes(fixture)).toEqual([8]);
      click(rows(fixture)[9], { shiftKey: true });
      await settle(fixture);
      expect(selectedIndexes(fixture)).toEqual([8, 9]);
      store.select(null);
      await settle(fixture);
      expect(selectedIndexes(fixture)).toEqual([]);
      click(rows(fixture)[0]);
      click(rows(fixture)[2], { shiftKey: true });
      await settle(fixture);
      store.select(store.selected() && { ...store.selected()!, id: 10 });
      await settle(fixture);
      expect(selectedIndexes(fixture)).toEqual([10]);
    });
  });

  it('paints rows that a highlight rule matches with a colour bar and a tint', async () => {
    const { fixture } = await render();
    const highlights = TestBed.inject(HighlightRules);
    expect(all(fixture, '.row.hl')).toHaveLength(0);
    highlights.add({ field: '@level', op: 'eq', value: 'Error', color: '#f2545b' });
    await settle(fixture);
    const painted = all<HTMLElement>(fixture, '.row.hl');
    expect(painted).toHaveLength(1);
    expect(painted[0].style.getPropertyValue('--hl')).toBe('#f2545b');
    expect(painted[0].textContent).toContain('ERR');
    painted[0].click();
    await settle(fixture);
    expect(painted[0].classList.contains('sel')).toBe(true);
    highlights.update(highlights.items()[0].id, { enabled: false });
    await settle(fixture);
    expect(all(fixture, '.row.hl')).toHaveLength(0);
    expect(rows(fixture).every((r) => !r.style.getPropertyValue('--hl'))).toBe(true);
  });

  it('re-queries when the filter or sort changes', async () => {
    const { fixture, store } = await render();
    store.toggleLevel('Error');
    await settle(fixture);
    expect(rows(fixture)).toHaveLength(1);
    store.toggleLevel('Error');
    buttonByText(fixture, /Oldest first/).click();
    await settle(fixture);
    expect(store.descending()).toBe(true);
    expect(rows(fixture)[0].textContent).toContain('line without a timestamp');
    el<HTMLButtonElement>(fixture, '.hcell .sort').click();
    await settle(fixture);
    expect(store.descending()).toBe(false);
    store.addFilter({ field: 'MachineName', op: 'eq', value: 'none' });
    await settle(fixture);
    expect(text(fixture)).toContain('No events match');
  });

  it('jumps to anchors and reports hidden ones', async () => {
    const { fixture, store } = await render();
    store.jumpTo(6);
    await settle(fixture);
    expect(store.selected()?.id).toBe(6);
    store.toggleLevel('Fatal');
    await settle(fixture);
    store.jumpTo(0);
    await settle(fixture);
    expect(store.anchorMissed()).toBe(true);
    expect(text(fixture)).toContain('hidden by the current filters');
  });

  it('adds and removes property columns', async () => {
    const { fixture, store } = await render();
    buttonByText(fixture, /Columns/).click();
    await settle(fixture);
    const filter = el<HTMLInputElement>(fixture, '.cols input.input');
    filter.value = 'machine';
    filter.dispatchEvent(new Event('input'));
    await settle(fixture);
    const boxes = all<HTMLInputElement>(fixture, '.col-opt input');
    expect(boxes).toHaveLength(1);
    boxes[0].click();
    await settle(fixture);
    expect(store.extraColumns()).toEqual(['MachineName']);
    expect(all(fixture, '.row .extra')[0].textContent).toBe('WEB01');
    all<HTMLInputElement>(fixture, '.col-opt input')[0].click();
    expect(store.extraColumns()).toEqual([]);
    store.addColumn('@source');
    store.addColumn('@line');
    store.addColumn('@template');
    await settle(fixture);
    const cells = all(fixture, '.row')[0].querySelectorAll('.extra');
    expect([...cells].map((c) => c.textContent)).toEqual(['app.clef', '1', 'Application starting — {Application} {Version} ({Framework})']);
    el<HTMLButtonElement>(fixture, '.hcell.extra .x').click();
    expect(store.extraColumns()).toEqual(['@line', '@template']);
  });

  it('virtualises huge results and compresses the scrollbar', async () => {
    const backend = new HugeBackend(2_000_000);
    const { fixture } = await render(backend);
    const vp = el(fixture, '.viewport');
    const spacer = vp.lastElementChild as HTMLElement;
    expect(parseFloat(spacer.style.height)).toBeLessThanOrEqual(10_000_000);
    expect(rows(fixture).length).toBeLessThan(40);
    vp.scrollTop = parseFloat(spacer.style.height);
    vp.dispatchEvent(new Event('scroll'));
    await settle(fixture);
    const last = rows(fixture).at(-1)!;
    expect(last.textContent).toContain('row 1999999');
    expect(backend.requests.some((r) => r.offset === 3999 * 500)).toBe(true);
  });

  it('loads pages on demand while selecting with the keyboard', async () => {
    const backend = new HugeBackend(5000);
    const { fixture, store } = await render(backend);
    const vp = el(fixture, '.viewport');
    vp.dispatchEvent(new KeyboardEvent('keydown', { key: 'End' }));
    await settle(fixture);
    await new Promise((r) => setTimeout(r, 120));
    expect(store.selected()?.id).toBe(4999);
    store.jumpTo(2500);
    await settle(fixture);
    expect(store.selected()?.id).toBe(2500);
  });

  it('reports page errors', async () => {
    const backend = tinyBackend();
    const { fixture, store } = await render(backend);
    const query = vi.spyOn(backend, 'queryPage').mockRejectedValueOnce('page failed');
    store.toggleLevel('Error');
    await settle(fixture);
    expect(el(fixture, '.err-card[role=alert]').textContent).toContain('page failed');
    expect(store.error()).toBeNull();
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(query).toHaveBeenCalledTimes(2);
    expect(all(fixture, '.err-card')).toHaveLength(0);
    expect(rows(fixture)).toHaveLength(1);
  });
});

describe('EventsGrid at scale', { timeout: 30_000 }, () => {
  const ROW = 26;
  const BIG = 50_000_000;
  let restore: PropertyDescriptor | undefined;
  beforeAll(() => {
    restore = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight');
    Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
      configurable: true,
      get(this: HTMLElement) {
        return this.classList?.contains('viewport') ? VIEWPORT_H + 28 : 0;
      },
    });
  });
  afterAll(() => {
    if (restore) Object.defineProperty(HTMLElement.prototype, 'clientHeight', restore);
  });
  beforeEach(() => localStorage.clear());

  function synthetic(total = BIG, extra: ConstructorParameters<typeof MockBackend>[0] = {}) {
    return new MockBackend({ latency: false, dataset: tinyDataset, syntheticTotal: total, ...extra });
  }

  async function render(backend: ImtBackend) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(EventsGrid);
    const store = TestBed.inject(WorkspaceStore);
    await settle(fixture);
    const grid = fixture.componentInstance as unknown as { virtualTop(): number; pages: Map<number, unknown>; total(): number; geometry(): { maxTop: number; maxScroll: number; scaled: boolean } };
    const vp = el(fixture, '.viewport');
    return { fixture, store, grid, vp };
  }

  const visibleIds = (f: Awaited<ReturnType<typeof render>>['fixture']) =>
    all(f, '.row:not(.placeholder)').map((r) => Number(/Synthetic event (\d+)/.exec(r.textContent ?? '')?.[1]));
  const key = (vp: HTMLElement, k: string) => vp.dispatchEvent(new KeyboardEvent('keydown', { key: k, cancelable: true }));
  const flush = async (f: Awaited<ReturnType<typeof render>>['fixture']) => {
    await settle(f);
    await new Promise((r) => setTimeout(r, 30));
    await settle(f);
  };

  it('shows the full total and a capped spacer for 50 million rows', async () => {
    const { fixture, grid, vp } = await render(synthetic());
    expect(el(fixture, '.total').textContent).toBe('50,000,000');
    expect(grid.geometry().scaled).toBe(true);
    expect(parseFloat((vp.lastElementChild as HTMLElement).style.height)).toBe(10_000_000 - VIEWPORT_H);
    expect(visibleIds(fixture)[0]).toBe(0);
  });

  it('jumps to the end with the scrollbar and shows the very last rows', async () => {
    const { fixture, grid, vp } = await render(synthetic());
    vp.scrollTop = grid.geometry().maxScroll;
    vp.dispatchEvent(new Event('scroll'));
    await flush(fixture);
    expect(grid.virtualTop()).toBe(grid.geometry().maxTop);
    const ids = visibleIds(fixture);
    expect(ids.at(-1)).toBe(BIG - 1);
  });

  it('moves the scrollbar thumb proportionally', async () => {
    const { fixture, grid, vp } = await render(synthetic());
    vp.scrollTop = grid.geometry().maxScroll / 2;
    vp.dispatchEvent(new Event('scroll'));
    await flush(fixture);
    expect(Math.abs(grid.virtualTop() / ROW - BIG / 2)).toBeLessThan(VIEWPORT_H / ROW + 1);
    expect(visibleIds(fixture).some((i) => Math.abs(i - BIG / 2) < 40)).toBe(true);
  });

  it('lands exactly on an anchor near 40 million and selects it', async () => {
    const { fixture, store, grid } = await render(synthetic());
    const id = 40_000_123;
    store.jumpTo(id);
    await flush(fixture);
    expect(store.selected()?.id).toBe(id);
    expect(visibleIds(fixture)).toContain(id);
    const row = all(fixture, '.row.sel')[0];
    expect(row).toBeDefined();
    const top = parseFloat(row.style.top);
    expect(top).toBeGreaterThan(0);
    expect(top + ROW).toBeLessThanOrEqual(VIEWPORT_H);
    // Centred on the row, within half a row.
    expect(Math.abs(top - (VIEWPORT_H - ROW) / 2)).toBeLessThan(ROW);
    expect(grid.virtualTop()).toBeCloseTo(id * ROW - VIEWPORT_H / 2 + ROW / 2, 3);
  });

  it('lands on an anchor in descending order too', async () => {
    const { fixture, store } = await render(synthetic());
    store.toggleSort();
    await flush(fixture);
    expect(visibleIds(fixture)[0]).toBe(BIG - 1);
    store.jumpTo(10_000_000);
    await flush(fixture);
    expect(store.selected()?.id).toBe(10_000_000);
    expect(visibleIds(fixture)).toContain(10_000_000);
  });

  it('navigates by keyboard: arrows, pages, Home and End', async () => {
    const { fixture, store, grid, vp } = await render(synthetic());
    key(vp, 'End');
    await flush(fixture);
    expect(store.selected()?.id).toBe(BIG - 1);
    expect(grid.virtualTop()).toBe(grid.geometry().maxTop);
    expect(visibleIds(fixture).at(-1)).toBe(BIG - 1);
    key(vp, 'ArrowUp');
    expect(store.selected()?.id).toBe(BIG - 2);
    key(vp, 'PageUp');
    await flush(fixture);
    expect(store.selected()?.id).toBe(BIG - 2 - 9);
    key(vp, 'Home');
    await flush(fixture);
    expect(store.selected()?.id).toBe(0);
    expect(grid.virtualTop()).toBe(0);
    key(vp, 'ArrowDown');
    key(vp, 'PageDown');
    await flush(fixture);
    expect(store.selected()?.id).toBe(1 + 9);
    for (let i = 0; i < 40; i++) key(vp, 'ArrowDown');
    await flush(fixture);
    expect(store.selected()?.id).toBe(50);
    // The selection stays on screen as the rows scroll under it.
    const sel = all(fixture, '.row.sel')[0];
    expect(parseFloat(sel.style.top) + ROW).toBeLessThanOrEqual(VIEWPORT_H + 0.5);
  });

  it('steps the wheel by pixels of the true content, not of the compressed spacer', async () => {
    const { fixture, grid, vp } = await render(synthetic());
    const wheel = (deltaY: number, deltaMode = 0, ctrlKey = false) => {
      const e = new WheelEvent('wheel', { deltaY, deltaMode, ctrlKey, cancelable: true });
      vp.dispatchEvent(e);
      return e;
    };
    const e = wheel(52);
    expect(e.defaultPrevented).toBe(true);
    expect(grid.virtualTop()).toBe(52);
    wheel(104);
    expect(grid.virtualTop()).toBe(156);
    wheel(1, 1);
    expect(grid.virtualTop()).toBe(156 + 3 * ROW);
    wheel(-100000);
    expect(grid.virtualTop()).toBe(0);
    expect(wheel(0).defaultPrevented).toBe(false);
    expect(wheel(10, 0, true).defaultPrevented).toBe(false);
    await flush(fixture);
    expect(visibleIds(fixture)[0]).toBe(0);
  });

  it('leaves the wheel to the browser while the content fits', async () => {
    const { vp } = await render(synthetic(1000));
    const e = new WheelEvent('wheel', { deltaY: 40, cancelable: true });
    vp.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(false);
  });

  it('does not let rounding of the scrollbar position move a row-exact position', async () => {
    const { fixture, grid, vp } = await render(synthetic());
    const e = new WheelEvent('wheel', { deltaY: 30, cancelable: true });
    vp.dispatchEvent(e);
    const top = grid.virtualTop();
    vp.dispatchEvent(new Event('scroll'));
    expect(grid.virtualTop()).toBe(top);
    // A scroll the user caused (somewhere else) is followed.
    vp.scrollTop = 5_000_000;
    vp.dispatchEvent(new Event('scroll'));
    await flush(fixture);
    expect(grid.virtualTop()).toBeGreaterThan(1e8);
  });

  it('keeps memory bounded while scrolling through millions of rows', async () => {
    const { fixture, grid, vp } = await render(synthetic());
    for (let i = 1; i <= 50; i++) {
      vp.scrollTop = i * 150_000;
      vp.dispatchEvent(new Event('scroll'));
      await settle(fixture, 2);
    }
    await flush(fixture);
    expect(grid.pages.size).toBeLessThanOrEqual(40);
    expect(visibleIds(fixture).length).toBeGreaterThan(10);
  });

  it('shows an inline error with Retry when jumping fails, and when the first page cannot load', async () => {
    const backend = synthetic();
    const { fixture, store } = await render(backend);
    vi.spyOn(backend, 'queryPage').mockRejectedValueOnce('jump failed');
    store.jumpTo(5);
    await flush(fixture);
    expect(el(fixture, '.err-card').textContent).toContain('jump failed');
    buttonByText(fixture, 'Retry').click();
    await flush(fixture);
    expect(all(fixture, '.err-card')).toHaveLength(0);
  });

  it('retries the first page when nothing has loaded yet', async () => {
    const backend = synthetic(BIG, { fail: { queryPage: 1 } });
    const { fixture } = await render(backend);
    expect(el(fixture, '.err-card[role=alert]').textContent).toContain('Simulated queryPage failure');
    expect(el(fixture, '.total').textContent).toBe('0');
    buttonByText(fixture, 'Retry').click();
    await flush(fixture);
    expect(all(fixture, '.err-card')).toHaveLength(0);
    expect(el(fixture, '.total').textContent).toBe('50,000,000');
  });

  it('retries a page that failed while scrolling', async () => {
    const backend = synthetic();
    const { fixture, grid, vp } = await render(backend);
    vi.spyOn(backend, 'queryPage').mockRejectedValueOnce('far page failed');
    vp.scrollTop = 4_000_000;
    vp.dispatchEvent(new Event('scroll'));
    await flush(fixture);
    expect(el(fixture, '.err-card').textContent).toContain('far page failed');
    buttonByText(fixture, 'Retry').click();
    await flush(fixture);
    expect(all(fixture, '.err-card')).toHaveLength(0);
    expect(visibleIds(fixture).length).toBeGreaterThan(10);
    expect(grid.virtualTop()).toBeGreaterThan(0);
  });

  describe('live tail', () => {
    async function live(descending: boolean, scrollRows: number | 'end') {
      const backend = synthetic(5000, { liveMs: 25 });
      const { fixture, store, grid, vp } = await render(backend);
      if (descending) store.toggleSort();
      await flush(fixture);
      if (scrollRows === 'end') key(vp, 'End');
      else if (scrollRows > 0) {
        vp.scrollTop = scrollRows * ROW;
        vp.dispatchEvent(new Event('scroll'));
      }
      await flush(fixture);
      const before = { total: grid.total(), top: grid.virtualTop(), ids: visibleIds(fixture) };
      const tail = TestBed.inject(LiveTail);
      await tail.toggle();
      expect(tail.enabled()).toBe(true);
      await vi.waitFor(() => expect(grid.total()).toBeGreaterThan(before.total + 1), { timeout: 3000 });
      await tail.toggle();
      await flush(fixture);
      return { fixture, store, grid, before, tail };
    }

    it('keeps the position when scrolled away in ascending order', async () => {
      const { grid, before, fixture } = await live(false, 1000);
      expect(grid.virtualTop()).toBe(before.top);
      expect(visibleIds(fixture)[0]).toBe(before.ids[0]);
    });

    it('keeps the same events in view in descending order when scrolled away (new events are inserted above)', async () => {
      const { grid, before, fixture } = await live(true, 1000);
      expect(grid.total()).toBeGreaterThan(before.total);
      expect(grid.virtualTop()).toBeGreaterThan(before.top);
      expect(visibleIds(fixture)).toContain(before.ids[Math.floor(before.ids.length / 2)]);
    });

    it('follows new events at the top in descending order', async () => {
      const { grid, fixture, before } = await live(true, 0);
      expect(grid.virtualTop()).toBe(0);
      expect(visibleIds(fixture)[0]).toBeGreaterThan(before.ids[0]);
    });

    it('follows new events at the end in ascending order', async () => {
      const { grid, fixture } = await live(false, 'end');
      expect(grid.virtualTop()).toBe(grid.geometry().maxTop);
      expect(visibleIds(fixture).at(-1)).toBe(grid.total() - 1);
    });

    it('has no live control of its own (the status bar owns it)', async () => {
      const { fixture } = await render(synthetic(5000));
      expect(all(fixture, 'button').some((b) => /live/i.test(b.textContent ?? ''))).toBe(false);
    });

    it('keeps the same rows selected when new events are inserted above them', async () => {
      const backend = synthetic(5000, { liveMs: 25 });
      const { fixture, store, grid, vp } = await render(backend);
      store.toggleSort();
      await flush(fixture);
      vp.scrollTop = 1000 * ROW;
      vp.dispatchEvent(new Event('scroll'));
      await flush(fixture);
      const visible = all<HTMLElement>(fixture, '.row:not(.placeholder)');
      visible[3].click();
      visible[5].dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true }));
      await flush(fixture);
      const selectedIds = () =>
        all(fixture, '.row[aria-selected=true]').map((r) => Number(/Synthetic event (\d+)/.exec(r.textContent ?? '')?.[1]));
      const before = selectedIds();
      expect(before).toHaveLength(3);
      const total = grid.total();
      const tail = TestBed.inject(LiveTail);
      await tail.toggle();
      await vi.waitFor(() => expect(grid.total()).toBeGreaterThan(total + 1), { timeout: 3000 });
      await tail.toggle();
      await flush(fixture);
      expect(selectedIds()).toEqual(before);
    });
  });
});
