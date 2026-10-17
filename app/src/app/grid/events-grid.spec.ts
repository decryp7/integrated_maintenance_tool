import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { Page, PageRequest, RecordView } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { MockBackend } from '../core/mock/mock-backend';
import { WorkspaceStore } from '../core/workspace.store';
import { SettingsStore } from '../settings/settings.store';
import { EventsGrid } from './events-grid';

const VIEWPORT_H = 260;

function fakeRow(i: number): RecordView {
  return { id: i, ts: 1_700_000_000_000 + i, level: 'Information', message: `row ${i}`, fields: { n: i }, source: 0, sourceName: 's.log', line: i + 1, group: 'g', markers: [] };
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
    expect(parseFloat(spacer.style.height)).toBeLessThanOrEqual(15_000_000);
    expect(rows(fixture).length).toBeLessThan(40);
    vp.scrollTop = 15_000_000 - VIEWPORT_H;
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
    vi.spyOn(backend, 'queryPage').mockRejectedValue('page failed');
    store.toggleLevel('Error');
    await settle(fixture);
    expect(store.error()).toBe('page failed');
  });
});
