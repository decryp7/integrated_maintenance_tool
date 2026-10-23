import { ComponentFixture, TestBed } from '@angular/core/testing';
import { afterEach, vi } from 'vitest';
import { all, buttonByText, el, settle, text, typeInto } from '../../../testing/helpers';
import { HeapObjectsPage, HeapPaths, HeapTypesPage } from '../../core/api.types';
import { MockProfilingBackend, MockProfilingOptions } from '../../core/mock/mock-profiling';
import { CapturesStore } from '../captures.store';
import { ProfileViewState } from '../profile-view-state.store';
import { openDemoCaptures, setupProfiling } from '../testing/profiling-test-support';
import { HeapSelection, HeapTab, sparkline } from './heap-selection';
import { flattenDominators } from './heap-dominators';
import { byTaken } from './heap-diff';
import { HeapView } from './heap-view';
import { at } from '../../../testing/must';

const HEAP_PATHS = ['/demo/profiles/heap-1.heapsnapshot', '/demo/profiles/heap-2.heapsnapshot', '/demo/profiles/heap-3.heapsnapshot'];

interface Setup {
  options?: MockProfilingOptions;
  open?: string[] | false;
  noGraph?: boolean;
  /** The tab to start on (the view starts on Findings; most tests look at Types). */
  tab?: HeapTab;
}

async function mount(s: Setup = {}) {
  const doubles = setupProfiling({ options: s.options });
  const backend = doubles.backend as MockProfilingBackend;
  if (s.noGraph) {
    const list = backend.list.bind(backend);
    vi.spyOn(backend, 'list').mockImplementation(async () => (await list()).map((c) => (c.capture === 'heap' ? { ...c, hasGraph: false } : c)));
  }
  const orig = {
    types: backend.heapTypes.bind(backend),
    dominators: backend.heapDominators.bind(backend),
  };
  const spies = {
    types: vi.spyOn(backend, 'heapTypes'),
    diff: vi.spyOn(backend, 'heapDiff'),
    growth: vi.spyOn(backend, 'heapGrowth'),
    dominators: vi.spyOn(backend, 'heapDominators'),
    instances: vi.spyOn(backend, 'heapInstances'),
    paths: vi.spyOn(backend, 'heapPaths'),
  };
  if (s.open === undefined) await openDemoCaptures();
  else if (s.open) await TestBed.inject(CapturesStore).openPaths(s.open);
  const state = TestBed.inject(ProfileViewState);
  const fixture: ComponentFixture<HeapView> = TestBed.createComponent(HeapView);
  fixture.debugElement.injector.get(HeapSelection).tab.set(s.tab ?? 'types');
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, state, backend, spies, orig };
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const tabs = (f: ComponentFixture<unknown>) => all<HTMLButtonElement>(f, '[role=tab]');
const tab = (f: ComponentFixture<unknown>, label: string) => tabs(f).find((t) => t.textContent!.trim() === label)!;

async function goto(f: ComponentFixture<unknown>, label: string): Promise<void> {
  tab(f, label).click();
  await settle(f, 6);
}

const typeRows = (f: ComponentFixture<unknown>) => all(f, 'table[aria-label=Types] tbody tr');
const cells = (row: Element) => [...row.querySelectorAll('td')].map((c) => c.textContent!.replace(/\s+/g, ' ').trim());
const select = (f: ComponentFixture<unknown>, label: string) => el<HTMLSelectElement>(f, `select[aria-label="${label}"]`);

function choose(sel: HTMLSelectElement, value: string): void {
  sel.value = value;
  sel.dispatchEvent(new Event('change'));
}

function key(target: Element, k: string): KeyboardEvent {
  const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true });
  target.dispatchEvent(e);
  return e;
}

describe('pure helpers', () => {
  it('orders snapshots by time taken, then id', () => {
    const h = (id: number, takenMs: number | null) => ({ id, takenMs }) as Parameters<typeof byTaken>[0][number];
    expect(byTaken([h(3, 30), h(1, 30), h(2, null)]).map((x) => x.id)).toEqual([2, 1, 3]);
  });

  it('draws a sparkline, flat series in the middle', () => {
    expect(sparkline([])).toBe('');
    expect(sparkline([5, 5, 5])).toBe('0.0,10.0 40.0,10.0 80.0,10.0');
    expect(sparkline([0, 10])).toBe('0.0,19.0 80.0,1.0');
    expect(sparkline([7])).toBe('0.0,10.0');
  });

  it('flattens only the expanded dominator levels that have children', () => {
    const row = (object: number, childCount: number) => ({ object, typeName: 'T', shallow: 1, retained: 1, childCount });
    const levels = new Map([
      [null, { rows: [row(1, 2), row(2, 0)], total: 2, loading: false, error: null }],
      [1, { rows: [row(3, 0)], total: 1, loading: false, error: null }],
    ]);
    expect(flattenDominators(levels, new Set()).map((r) => r.row.object)).toEqual([1, 2]);
    const open = flattenDominators(levels, new Set([1, 2]));
    expect(open.map((r) => [r.row.object, r.level, r.pos, r.size, r.expanded])).toEqual([
      [1, 1, 1, 2, true],
      [3, 2, 1, 1, false],
      [2, 1, 2, 2, false],
    ]);
    expect(flattenDominators(new Map(), new Set())).toEqual([]);
  });
});

describe('HeapView', () => {
  afterEach(() => vi.restoreAllMocks());

  it('asks to open a heap snapshot', async () => {
    const { fixture, spies } = await mount({ open: false });
    expect(text(fixture)).toContain('Open a heap snapshot (.heapsnapshot) to analyse memory.');
    expect(spies.types).not.toHaveBeenCalled();
  });

  it('summarises the active snapshot and lists the snapshots in a select', async () => {
    const { fixture } = await mount();
    const names = all<HTMLOptionElement>(fixture, 'select[aria-label="Heap snapshot"] option').map((o) => o.textContent);
    expect(names).toEqual(['heap-1.heapsnapshot', 'heap-2.heapsnapshot', 'heap-3.heapsnapshot']);
    expect(text(fixture)).toMatch(/\d+ objects · \d+ types · /);
    expect(text(fixture)).not.toContain('no object graph');
  });

  it('keeps exact byte counts in the tooltips of humanized sizes', async () => {
    const { fixture } = await mount();
    const titles = all<HTMLElement>(fixture, 'td.num[title]').map((e) => e.title);
    expect(titles.length).toBeGreaterThan(0);
    expect(titles.every((t) => /bytes?$/.test(t))).toBe(true);
  });

  it('switches snapshot, re-querying the types and showing its import notes', async () => {
    const { fixture, state, spies } = await mount();
    expect(text(fixture)).not.toContain('Weak and shortcut edges');
    choose(select(fixture, 'Heap snapshot'), '5');
    await settle(fixture, 6);
    expect(state.heapId()).toBe(5);
    expect(spies.types.mock.calls.at(-1)![0].snapshotId).toBe(5);
    expect(el(fixture, '[role=note]').textContent).toContain('Weak and shortcut edges were skipped.');
  });

  it('drops the selected object when another snapshot is picked', async () => {
    const { fixture } = await mount();
    await goto(fixture, 'Instances and paths');
    typeInto(el<HTMLInputElement>(fixture, 'input[aria-label="Type name"]'), 'Shop.Orders.Order');
    el<HTMLFormElement>(fixture, 'form').dispatchEvent(new Event('submit', { cancelable: true }));
    await settle(fixture, 6);
    el<HTMLButtonElement>(fixture, 'table[aria-label="Instances by retained size"] button').click();
    await settle(fixture, 6);
    expect(all(fixture, '[data-testid=root-path]').length).toBeGreaterThan(0);
    choose(select(fixture, 'Heap snapshot'), '4');
    await settle(fixture, 6);
    expect(all(fixture, '[data-testid=root-path]')).toHaveLength(0);
    expect(text(fixture)).toContain('Choose an object to see the references');
  });

  describe('tabs', () => {
    it('exposes a tablist with the selected tab and a panel labelled by it', async () => {
      const { fixture } = await mount({ tab: 'findings' });
      expect(tabs(fixture).map((t) => t.textContent!.trim())).toEqual(['Findings', 'Types', 'Compare snapshots', 'Growth', 'Dominator tree', 'Instances and paths']);
      expect(tabs(fixture).map((t) => t.getAttribute('aria-selected'))).toEqual(['true', 'false', 'false', 'false', 'false', 'false']);
      expect(tabs(fixture).map((t) => t.getAttribute('tabindex'))).toEqual(['0', '-1', '-1', '-1', '-1', '-1']);
      expect(el(fixture, '[role=tabpanel]').getAttribute('aria-labelledby')).toBe('heap-tab-findings');
      await goto(fixture, 'Growth');
      expect(tab(fixture, 'Growth').getAttribute('aria-selected')).toBe('true');
      expect(el(fixture, '[role=tabpanel]').getAttribute('aria-labelledby')).toBe('heap-tab-growth');
    });

    it('moves between tabs with Left and Right, wrapping around, and focuses the new tab', async () => {
      const { fixture } = await mount();
      const list = el(fixture, '[role=tablist]');
      expect(key(list, 'ArrowRight').defaultPrevented).toBe(true);
      fixture.detectChanges();
      expect(tab(fixture, 'Compare snapshots').getAttribute('aria-selected')).toBe('true');
      await settle(fixture, 2);
      key(list, 'ArrowLeft');
      key(list, 'ArrowLeft');
      key(list, 'ArrowLeft');
      fixture.detectChanges();
      expect(tab(fixture, 'Instances and paths').getAttribute('aria-selected')).toBe('true');
      key(list, 'ArrowRight');
      fixture.detectChanges();
      expect(tab(fixture, 'Findings').getAttribute('aria-selected')).toBe('true');
      expect(key(list, 'Home').defaultPrevented).toBe(false);
    });

    it('skips the disabled dominator tab with the arrow keys (regression)', async () => {
      const { fixture } = await mount({ noGraph: true });
      await goto(fixture, 'Growth');
      key(el(fixture, '[role=tablist]'), 'ArrowRight');
      fixture.detectChanges();
      expect(tab(fixture, 'Instances and paths').getAttribute('aria-selected')).toBe('true');
      key(el(fixture, '[role=tablist]'), 'ArrowLeft');
      fixture.detectChanges();
      expect(tab(fixture, 'Growth').getAttribute('aria-selected')).toBe('true');
    });

    it('disables the dominator tree without an object graph, and says so', async () => {
      const { fixture } = await mount({ noGraph: true });
      expect(tab(fixture, 'Dominator tree').disabled).toBe(true);
      expect(tab(fixture, 'Growth').disabled).toBe(false);
      expect(text(fixture)).toContain('no object graph (types only)');
    });
  });

  describe('types', () => {
    it('requests the largest types by bytes and renders count, bytes, retained and share', async () => {
      const { fixture, spies } = await mount();
      expect(spies.types.mock.calls.at(-1)![0]).toEqual({ snapshotId: 3, sort: 'bytes', search: null, offset: 0, limit: 200 });
      const rows = typeRows(fixture);
      expect(rows.length).toBeGreaterThan(3);
      const first = cells(at(rows, 0));
      expect(first[0]).not.toBe('');
      expect(first[1]).toMatch(/\d/);
      expect(first[3]).not.toBe('—');
      expect(first[4]).toMatch(/%$/);
      expect(text(fixture)).toMatch(/\d+ types · \d[\d,. ]* objects · /);
    });

    it('sorts by the clicked column and reports it in aria-sort', async () => {
      const { fixture, spies } = await mount();
      const th = (label: string) => all(fixture, 'table[aria-label=Types] th').find((h) => h.textContent!.trim().startsWith(label))!;
      expect(th('Bytes').getAttribute('aria-sort')).toBe('descending');
      expect(th('Count').getAttribute('aria-sort')).toBeNull();
      th('Count').querySelector('button')!.click();
      await settle(fixture, 6);
      expect(spies.types.mock.calls.at(-1)![0].sort).toBe('count');
      expect(th('Count').getAttribute('aria-sort')).toBe('descending');
      expect(th('Count').textContent).toContain('▼');
      th('Type').querySelector('button')!.click();
      await settle(fixture, 6);
      expect(th('Type').getAttribute('aria-sort')).toBe('ascending');
      expect(th('Type').textContent).toContain('▲');
      th('Retained').querySelector('button')!.click();
      await settle(fixture, 6);
      expect(spies.types.mock.calls.at(-1)![0].sort).toBe('retained');
    });

    it('debounces the type search and reports no match', async () => {
      const { fixture, spies } = await mount();
      const calls = spies.types.mock.calls.length;
      const input = el<HTMLInputElement>(fixture, 'input[aria-label="Search types"]');
      typeInto(input, ' Ord');
      typeInto(input, ' Customer ');
      expect(spies.types.mock.calls.length).toBe(calls);
      await wait(260);
      await settle(fixture, 6);
      expect(spies.types.mock.calls.at(-1)![0].search).toBe('Customer');
      expect(typeRows(fixture).map((r) => cells(r)[0])).toEqual(['Shop.Orders.Customer']);
      typeInto(input, 'no-such-type-xyz');
      await wait(260);
      await settle(fixture, 6);
      expect(text(fixture)).toContain('No types match.');
    });

    it('disables retained sorting and shows a dash for a types-only snapshot', async () => {
      const { fixture, spies, orig } = await mount();
      spies.types.mockImplementation(async (r) => {
        const p = await orig.types(r);
        return { ...p, hasGraph: false, rows: p.rows.map((x) => ({ ...x, retained: null })) } satisfies HeapTypesPage;
      });
      choose(select(fixture, 'Heap snapshot'), '4');
      await settle(fixture, 6);
      const retained = all<HTMLButtonElement>(fixture, 'table[aria-label=Types] th button').find((b) => b.textContent!.startsWith('Retained'))!;
      expect(retained.disabled).toBe(true);
      expect(cells(at(typeRows(fixture), 0))[3]).toBe('—');
    });

    function paged(backend: MockProfilingBackend, more: number) {
      const real = backend.heapTypes.bind(backend);
      return vi.spyOn(backend, 'heapTypes').mockImplementation(async (r) => {
        const p = await real({ ...r, offset: 0, limit: 200 });
        const rows = r.offset ? p.rows.slice(0, more).map((x, i) => ({ ...x, name: `Extra${i}` })) : p.rows;
        return { ...p, rows, totalRows: p.rows.length + more } satisfies HeapTypesPage;
      });
    }

    it('loads more rows', async () => {
      const doubles = setupProfiling();
      const spy = paged(doubles.backend as MockProfilingBackend, 2);
      await openDemoCaptures();
      const fixture = TestBed.createComponent(HeapView);
      fixture.debugElement.injector.get(HeapSelection).tab.set('types');
      fixture.detectChanges();
      await settle(fixture, 6);
      const before = typeRows(fixture).length;
      buttonByText(fixture, 'Load 2 more').click();
      await settle(fixture, 4);
      expect(spy.mock.calls.at(-1)![0]).toMatchObject({ offset: before, limit: 200 });
      expect(typeRows(fixture)).toHaveLength(before + 2);
      expect(all(fixture, 'button.more')).toHaveLength(0);
    });

    it('keeps the button when loading more fails', async () => {
      const doubles = setupProfiling();
      const spy = paged(doubles.backend as MockProfilingBackend, 2);
      await openDemoCaptures();
      const fixture = TestBed.createComponent(HeapView);
      fixture.debugElement.injector.get(HeapSelection).tab.set('types');
      fixture.detectChanges();
      await settle(fixture, 6);
      const before = typeRows(fixture).length;
      spy.mockRejectedValueOnce('boom');
      buttonByText(fixture, 'Load 2 more').click();
      await settle(fixture, 4);
      expect(typeRows(fixture)).toHaveLength(before);
      expect(buttonByText(fixture, 'Load 2 more')).toBeTruthy();
    });

    it('opens the instances of a clicked type', async () => {
      const { fixture, spies } = await mount();
      const name = cells(at(typeRows(fixture), 0))[0];
      at(typeRows(fixture), 0).querySelector<HTMLButtonElement>('button.link')!.click();
      await settle(fixture, 6);
      expect(tab(fixture, 'Instances and paths').getAttribute('aria-selected')).toBe('true');
      expect(spies.instances.mock.calls.at(-1)![0]).toMatchObject({ snapshotId: 3, typeName: name, limit: 200 });
      expect(el<HTMLInputElement>(fixture, 'input[aria-label="Type name"]').value).toBe(name);
    });

    it('shows the error with Retry', async () => {
      const { fixture } = await mount({ options: { fail: { heapTypes: 1 } } });
      expect(el(fixture, '[role=alert]').textContent).toContain('mock: heapTypes failed');
      buttonByText(fixture, 'Retry').click();
      await settle(fixture, 6);
      expect(typeRows(fixture).length).toBeGreaterThan(0);
    });
  });

  describe('compare snapshots', () => {
    const rowsOf = (f: ComponentFixture<unknown>) => all(f, 'table[aria-label^="Snapshot comparison"] tbody tr');

    it('defaults to the two newest snapshots', async () => {
      const { fixture, spies } = await mount();
      await goto(fixture, 'Compare snapshots');
      expect(spies.diff.mock.calls.at(-1)![0]).toEqual({ baselineId: 4, targetId: 5, sort: 'bytesDelta', limit: 500 });
      expect(select(fixture, 'Before snapshot').selectedOptions[0]?.textContent).toBe('heap-2.heapsnapshot');
      expect(select(fixture, 'After snapshot').selectedOptions[0]?.textContent).toBe('heap-3.heapsnapshot');
      expect(el(fixture, '[role=tabpanel] .bar .muted').textContent).toMatch(/→.*\(\+/);
      expect(rowsOf(fixture).length).toBeGreaterThan(0);
    });

    it('re-queries with other snapshots and flags new and gone types', async () => {
      const { fixture, spies } = await mount();
      await goto(fixture, 'Compare snapshots');
      choose(select(fixture, 'Before snapshot'), '3');
      choose(select(fixture, 'After snapshot'), '4');
      await settle(fixture, 6);
      expect(spies.diff.mock.calls.at(-1)![0]).toMatchObject({ baselineId: 3, targetId: 4 });
      const audit = rowsOf(fixture).find((r) => r.textContent!.includes('AuditLog'))!;
      expect(audit.querySelector('.tag.new')!.textContent).toBe('new');
      choose(select(fixture, 'Before snapshot'), '4');
      choose(select(fixture, 'After snapshot'), '3');
      await settle(fixture, 6);
      const gone = rowsOf(fixture).find((r) => r.textContent!.includes('AuditLog'))!;
      expect(gone.querySelector('.tag.gone')!.textContent).toBe('gone');
      expect(gone.querySelector('td.down')).not.toBeNull();
    });

    it('marks growth with arrows and up/down classes', async () => {
      const { fixture } = await mount();
      await goto(fixture, 'Compare snapshots');
      expect(all(fixture, 'td.up').length).toBeGreaterThan(0);
      expect(rowsOf(fixture).some((r) => r.textContent!.includes('▲'))).toBe(true);
    });

    it('sorts by the chosen order and opens instances of a type', async () => {
      const { fixture, spies } = await mount();
      await goto(fixture, 'Compare snapshots');
      choose(select(fixture, 'Sort by'), 'countDelta');
      await settle(fixture, 6);
      expect(spies.diff.mock.calls.at(-1)![0].sort).toBe('countDelta');
      const name = cells(at(rowsOf(fixture), 0))[0];
      at(rowsOf(fixture), 0).querySelector<HTMLButtonElement>('button.link')!.click();
      await settle(fixture, 6);
      expect(tab(fixture, 'Instances and paths').getAttribute('aria-selected')).toBe('true');
      expect(spies.instances.mock.calls.at(-1)![0].typeName).toBe(name);
    });

    it('asks for a second snapshot when fewer than two are open', async () => {
      const { fixture, spies } = await mount({ open: [at(HEAP_PATHS, 0)] });
      await goto(fixture, 'Compare snapshots');
      expect(text(fixture)).toContain('Open a second heap snapshot to compare.');
      expect(spies.diff).not.toHaveBeenCalled();
    });

    it('shows the error with Retry', async () => {
      const { fixture } = await mount({ options: { fail: { heapDiff: 1 } } });
      await goto(fixture, 'Compare snapshots');
      expect(el(fixture, '[role=alert]').textContent).toContain('heapDiff failed');
      buttonByText(fixture, 'Retry').click();
      await settle(fixture, 6);
      expect(rowsOf(fixture).length).toBeGreaterThan(0);
    });
  });

  describe('growth', () => {
    const rowsOf = (f: ComponentFixture<unknown>) => all(f, 'table[aria-label="Growth across snapshots"] tbody tr');

    it('analyses all three snapshots, flagging steady growers with a sparkline', async () => {
      const { fixture, spies } = await mount();
      await goto(fixture, 'Growth');
      expect(spies.growth.mock.calls.at(-1)![0]).toEqual({ snapshotIds: [3, 4, 5], steadyOnly: false, limit: 500 });
      expect(all<HTMLInputElement>(fixture, 'fieldset input').map((c) => c.checked)).toEqual([true, true, true]);
      expect(el(fixture, '[role=status]').textContent).toMatch(/\d+ types? grew in every snapshot/);
      const steady = rowsOf(fixture).filter((r) => r.querySelector('.tag.steady'));
      expect(steady.length).toBeGreaterThan(0);
      expect(steady[0]?.querySelector('.tag.steady')!.textContent).toContain('steady growth');
      const svg = at(steady, 0).querySelector('svg')!;
      const name = cells(at(steady, 0))[0] ?? '';
      expect(svg.getAttribute('aria-label')).toMatch(new RegExp(`^Trend of ${name.replace(/[.\\[\]]/g, '\\$&')}: .+ → .+ → .+$`));
      expect(svg.querySelector('polyline')!.getAttribute('points')).toMatch(/^\d/);
      expect(svg.querySelector('polyline')!.classList.contains('steady')).toBe(true);
      expect(steady[0]?.querySelector('td.up')).not.toBeNull();
    });

    it('can show steady growers only', async () => {
      const { fixture, spies } = await mount();
      await goto(fixture, 'Growth');
      const all0 = rowsOf(fixture).length;
      const box = all<HTMLInputElement>(fixture, '.bar input[type=checkbox]').find((c) => c.parentElement!.textContent!.includes('Steady growth only'))!;
      box.checked = true;
      box.dispatchEvent(new Event('change'));
      await settle(fixture, 6);
      expect(spies.growth.mock.calls.at(-1)![0].steadyOnly).toBe(true);
      expect(rowsOf(fixture).length).toBeLessThanOrEqual(all0);
      expect(rowsOf(fixture).every((r) => r.querySelector('.tag.steady'))).toBe(true);
    });

    it('needs three chosen snapshots and says how many more to open', async () => {
      const { fixture, spies } = await mount();
      await goto(fixture, 'Growth');
      const calls = spies.growth.mock.calls.length;
      const first = at(all<HTMLInputElement>(fixture, 'fieldset input'), 0);
      first.checked = false;
      first.dispatchEvent(new Event('change'));
      await settle(fixture, 6);
      expect(text(fixture)).toContain('Growth needs at least three heap snapshots.');
      expect(spies.growth.mock.calls.length).toBe(calls);
      first.checked = true;
      first.dispatchEvent(new Event('change'));
      await settle(fixture, 6);
      expect(rowsOf(fixture).length).toBeGreaterThan(0);
    });

    it('tells how many snapshots are missing', async () => {
      const { fixture } = await mount({ open: HEAP_PATHS.slice(0, 2) });
      await goto(fixture, 'Growth');
      expect(text(fixture)).toContain('Growth needs at least three heap snapshots: open 1 more.');
    });

    it('says so when no type grew, and shows errors with Retry', async () => {
      const { fixture, backend } = await mount({ options: { fail: { heapGrowth: 1 } } });
      await goto(fixture, 'Growth');
      expect(el(fixture, '[role=alert]').textContent).toContain('heapGrowth failed');
      vi.spyOn(backend, 'heapGrowth').mockResolvedValue({ rows: [], totalRows: 0, steadyCount: 1 });
      buttonByText(fixture, 'Retry').click();
      await settle(fixture, 6);
      expect(text(fixture)).toContain('No types grew.');
      expect(text(fixture)).toContain('1 type grew in every snapshot');
    });
  });

  describe('dominator tree', () => {
    const grid = (f: ComponentFixture<unknown>) => el(f, 'table[role=treegrid]');
    const rowsOf = (f: ComponentFixture<unknown>) => all(f, 'table[role=treegrid] tbody tr');

    it('loads the top level lazily as a treegrid', async () => {
      const { fixture, spies } = await mount();
      expect(spies.dominators).not.toHaveBeenCalled();
      await goto(fixture, 'Dominator tree');
      expect(spies.dominators.mock.calls.at(-1)![0]).toEqual({ snapshotId: 3, parent: null, limit: 100 });
      const rows = rowsOf(fixture);
      expect(rows.length).toBeGreaterThan(0);
      expect(rows.map((r) => r.getAttribute('aria-level'))).toEqual(rows.map(() => '1'));
      expect(rows[0]?.getAttribute('aria-posinset')).toBe('1');
      expect(rows[0]?.getAttribute('aria-setsize')).toBe(String(rows.length));
      expect(rows.map((r) => r.getAttribute('tabindex'))).toEqual(['0', ...rows.slice(1).map(() => '-1')]);
      expect(grid(fixture).getAttribute('aria-label')).toBe('Dominator tree');
    });

    async function openFirstParent(fixture: ComponentFixture<unknown>) {
      await goto(fixture, 'Dominator tree');
      const rows = rowsOf(fixture);
      const index = rows.findIndex((r) => r.hasAttribute('aria-expanded'));
      expect(index).toBeGreaterThanOrEqual(0);
      return index;
    }

    it('expands and collapses with the twisty, loading children once', async () => {
      const { fixture, spies } = await mount();
      const index = await openFirstParent(fixture);
      const topCount = rowsOf(fixture).length;
      const row = () => at(rowsOf(fixture), index);
      expect(row().getAttribute('aria-expanded')).toBe('false');
      row().querySelector<HTMLElement>('.tw')!.click();
      await settle(fixture, 6);
      expect(row().getAttribute('aria-expanded')).toBe('true');
      expect(spies.dominators.mock.calls.at(-1)![0].parent).not.toBeNull();
      const withChildren = rowsOf(fixture);
      expect(withChildren.length).toBeGreaterThan(topCount);
      expect(withChildren[index + 1]?.getAttribute('aria-level')).toBe('2');
      const calls = spies.dominators.mock.calls.length;
      row().querySelector<HTMLElement>('.tw')!.click();
      await settle(fixture, 3);
      expect(rowsOf(fixture)).toHaveLength(topCount);
      row().querySelector<HTMLElement>('.tw')!.click();
      await settle(fixture, 3);
      expect(rowsOf(fixture)).toHaveLength(withChildren.length);
      expect(spies.dominators.mock.calls.length).toBe(calls); // cached
    });

    it('retries a failed child level when it is expanded again (regression)', async () => {
      const { fixture, spies, orig } = await mount();
      const index = await openFirstParent(fixture);
      const row = () => at(rowsOf(fixture), index);
      spies.dominators.mockImplementationOnce(() => Promise.reject('children failed'));
      row().querySelector<HTMLElement>('.tw')!.click();
      await settle(fixture, 6);
      expect(text(fixture)).toContain('children failed');
      spies.dominators.mockImplementation(orig.dominators);
      row().querySelector<HTMLElement>('.tw')!.click();
      await settle(fixture, 3);
      row().querySelector<HTMLElement>('.tw')!.click();
      await settle(fixture, 6);
      expect(text(fixture)).not.toContain('children failed');
      expect(rowsOf(fixture)[index + 1]?.getAttribute('aria-level')).toBe('2');
    });

    it('ignores the twisty of a leaf', async () => {
      const { fixture, spies } = await mount();
      await goto(fixture, 'Dominator tree');
      const leaf = rowsOf(fixture).find((r) => !r.hasAttribute('aria-expanded'));
      if (!leaf) return;
      const calls = spies.dominators.mock.calls.length;
      leaf.querySelector<HTMLElement>('.tw')!.click();
      await settle(fixture, 3);
      expect(spies.dominators.mock.calls.length).toBe(calls);
    });

    it('navigates and expands with the keyboard', async () => {
      const { fixture, spies } = await mount();
      const index = await openFirstParent(fixture);
      const g = grid(fixture);
      const rows = () => rowsOf(fixture);
      // Move down to the first expandable row.
      for (let i = 0; i < index; i++) key(at(rows(), i), 'ArrowDown');
      await settle(fixture, 2);
      expect(rows()[index]?.getAttribute('tabindex')).toBe('0');
      expect(key(g, 'ArrowRight').defaultPrevented).toBe(true);
      await settle(fixture, 4);
      expect(rows()[index]?.getAttribute('aria-expanded')).toBe('true');
      expect(key(g, 'ArrowRight').defaultPrevented).toBe(false); // already open
      key(g, 'ArrowDown');
      await settle(fixture, 2);
      expect(rows()[index + 1]?.getAttribute('tabindex')).toBe('0');
      key(g, 'ArrowUp');
      await settle(fixture, 2);
      expect(rows()[index]?.getAttribute('tabindex')).toBe('0');
      expect(key(g, 'ArrowLeft').defaultPrevented).toBe(true);
      await settle(fixture, 2);
      expect(rows()[index]?.getAttribute('aria-expanded')).toBe('false');
      expect(key(g, 'ArrowLeft').defaultPrevented).toBe(false);
      expect(key(g, 'x').defaultPrevented).toBe(false);
      expect(spies.dominators.mock.calls.length).toBeGreaterThan(1);
    });

    it('opens the paths to roots with Enter and with the button', async () => {
      const { fixture, spies } = await mount();
      await goto(fixture, 'Dominator tree');
      const firstType = at(rowsOf(fixture), 0).querySelector('.ellipsis')!.textContent!;
      key(grid(fixture), 'Enter');
      await settle(fixture, 6);
      expect(tab(fixture, 'Instances and paths').getAttribute('aria-selected')).toBe('true');
      expect(el<HTMLInputElement>(fixture, 'input[aria-label="Type name"]').value).toBe(firstType);
      const object = spies.paths.mock.calls.at(-1)![0].object;
      expect(object).toBeGreaterThan(0);
      expect(el(fixture, '[data-testid=root-path]')).toBeTruthy();

      await goto(fixture, 'Dominator tree');
      const btn = at(rowsOf(fixture), 1).querySelector<HTMLButtonElement>('button')!;
      expect(btn.textContent).toBe('Paths to roots');
      expect(btn.getAttribute('aria-label')).toMatch(/^Why is .+ @\d+ alive\?$/);
      btn.click();
      await settle(fixture, 6);
      expect(tab(fixture, 'Instances and paths').getAttribute('aria-selected')).toBe('true');
      expect(spies.paths.mock.calls.at(-1)![0].object).not.toBe(object);
    });

    it('ignores keys when there are no rows', async () => {
      const { fixture, backend } = await mount();
      vi.spyOn(backend, 'heapDominators').mockResolvedValue({ rows: [], totalRows: 0, totalBytes: 0 });
      await goto(fixture, 'Dominator tree');
      expect(key(grid(fixture), 'ArrowDown').defaultPrevented).toBe(false);
    });

    it('shows a failed child load as a line and a failed top level with Retry', async () => {
      const { fixture, spies, orig } = await mount();
      spies.dominators.mockImplementation(async (r) => {
        if (r.parent !== null && r.parent !== undefined) throw 'children unavailable';
        return orig.dominators(r);
      });
      const index = await openFirstParent(fixture);
      at(rowsOf(fixture), index).querySelector<HTMLElement>('.tw')!.click();
      await settle(fixture, 6);
      expect(el(fixture, '.pad').textContent).toMatch(/^@\d+: children unavailable$/);
    });

    it('shows the loading line of children while they load', async () => {
      const { fixture, spies, orig } = await mount();
      let release: () => void = () => undefined;
      spies.dominators.mockImplementation(async (r) => {
        if (r.parent !== null && r.parent !== undefined) await new Promise<void>((res) => (release = res));
        return orig.dominators(r);
      });
      const index = await openFirstParent(fixture);
      at(rowsOf(fixture), index).querySelector<HTMLElement>('.tw')!.click();
      await settle(fixture, 3);
      expect(el(fixture, '.pad').textContent).toMatch(/^Loading children of @\d+…$/);
      release();
      await settle(fixture, 4);
      expect(fixture.nativeElement.querySelector('.pad')).toBeNull();
    });

    it('shows a failed top level with Retry', async () => {
      const { fixture } = await mount({ options: { fail: { heapDominators: 1 } } });
      await goto(fixture, 'Dominator tree');
      expect(el(fixture, '[role=alert]').textContent).toContain('heapDominators failed');
      buttonByText(fixture, 'Retry').click();
      await settle(fixture, 6);
      expect(rowsOf(fixture).length).toBeGreaterThan(0);
    });

    it('starts over for another snapshot', async () => {
      const { fixture, spies } = await mount();
      await goto(fixture, 'Dominator tree');
      choose(select(fixture, 'Heap snapshot'), '5');
      await settle(fixture, 6);
      expect(spies.dominators.mock.calls.at(-1)![0]).toMatchObject({ snapshotId: 5, parent: null });
      expect(rowsOf(fixture).length).toBeGreaterThan(0);
    });
  });

  describe('instances and paths', () => {
    const instanceRows = (f: ComponentFixture<unknown>) => all(f, 'table[aria-label="Instances by retained size"] tbody tr');

    async function showType(fixture: ComponentFixture<unknown>, name: string) {
      await goto(fixture, 'Instances and paths');
      typeInto(el<HTMLInputElement>(fixture, 'input[aria-label="Type name"]'), name);
      buttonByText(fixture, 'Show instances').click();
      await settle(fixture, 6);
    }

    it('asks to pick a type first', async () => {
      const { fixture, spies } = await mount();
      await goto(fixture, 'Instances and paths');
      expect(text(fixture)).toContain('Pick a type (in the Types tab, or type its name) to list its instances.');
      expect(text(fixture)).toContain('Choose an object to see the references that keep it alive.');
      expect(spies.instances).not.toHaveBeenCalled();
    });

    it('lists the instances of the typed type, ignoring blank input', async () => {
      const { fixture, spies } = await mount();
      await showType(fixture, '  ');
      expect(spies.instances).not.toHaveBeenCalled();
      await showType(fixture, '  Shop.Orders.Customer ');
      expect(spies.instances.mock.calls.at(-1)![0]).toEqual({ snapshotId: 3, typeName: 'Shop.Orders.Customer', limit: 200 });
      expect(instanceRows(fixture)).toHaveLength(30);
      expect(text(fixture)).toMatch(/30 instances · /);
      const retained = instanceRows(fixture).map((r) => cells(r)[1]);
      expect(retained.length).toBe(30);
      expect(instanceRows(fixture)[0]?.querySelector('button')!.getAttribute('aria-label')).toMatch(/^Why is Shop\.Orders\.Customer @\d+ alive\?$/);
    });

    it('says so when the type has no instances', async () => {
      const { fixture } = await mount();
      await showType(fixture, 'No.Such.Type');
      expect(text(fixture)).toContain('No instances of this type.');
    });

    it('shows the path from a GC root as an ordered chain with the edges', async () => {
      const { fixture, spies } = await mount();
      await showType(fixture, 'Shop.Orders.Customer');
      const button = at(instanceRows(fixture), 0).querySelector<HTMLButtonElement>('button')!;
      const object = Number(button.textContent!.replace('@', ''));
      button.click();
      await settle(fixture, 6);
      expect(spies.paths.mock.calls.at(-1)![0]).toEqual({ snapshotId: 3, object, maxPaths: 5 });
      expect(instanceRows(fixture)[0]?.classList.contains('sel')).toBe(true);
      const chains = all(fixture, '[data-testid=root-path]');
      expect(chains.length).toBeGreaterThan(0);
      const steps = [...at(chains, 0).querySelectorAll('li')];
      expect(chains[0]?.tagName).toBe('OL');
      expect(chains[0]?.getAttribute('aria-label')).toBe(`Path 1 from a GC root, ${steps.length} steps`);
      expect(steps[0]?.classList.contains('root')).toBe(true);
      expect(steps[0]?.querySelector('.tag')!.textContent).toBe('GC root');
      expect(steps[0]?.querySelector('.edge')).toBeNull();
      expect(steps.slice(1).every((s) => s.querySelector('.edge') && !s.querySelector('.tag'))).toBe(true);
      expect(steps.at(-1)!.classList.contains('target')).toBe(true);
      expect(steps.at(-1)!.textContent).toContain(`Shop.Orders.Customer @${object}`);
      expect(steps.at(-1)!.querySelector('.edge')!.textContent).toMatch(/^\.\[\d+\] →$/);
      expect(el(fixture, 'h3').textContent).toBe('Why is this alive?');
    });

    it('notes unreachable objects and truncated path lists', async () => {
      const { fixture, backend } = await mount();
      await showType(fixture, 'Shop.Orders.Customer');
      const spy = vi.spyOn(backend, 'heapPaths').mockResolvedValue({ paths: [], unreachable: true, truncated: true } satisfies HeapPaths);
      at(instanceRows(fixture), 0).querySelector<HTMLButtonElement>('button')!.click();
      await settle(fixture, 6);
      expect(spy).toHaveBeenCalled();
      expect(text(fixture)).toContain('Unreachable: nothing on a path from a GC root keeps it alive');
      expect(text(fixture)).toContain('More paths exist; the shortest are shown.');
      expect(all(fixture, '[data-testid=root-path]')).toHaveLength(0);
    });

    it('lists several paths with their own labels', async () => {
      const { fixture, backend } = await mount();
      await showType(fixture, 'Shop.Orders.Customer');
      const step = (object: number, typeName: string, edge: string | null) => ({ object, typeName, edge });
      vi.spyOn(backend, 'heapPaths').mockResolvedValue({
        paths: [
          [step(1, 'Root', null), step(2, 'A', '.x')],
          [step(1, 'Root', null), step(3, 'B', '.y'), step(2, 'A', '.z')],
        ],
        unreachable: false,
        truncated: false,
      });
      at(instanceRows(fixture), 0).querySelector<HTMLButtonElement>('button')!.click();
      await settle(fixture, 6);
      expect(all(fixture, '[data-testid=root-path]').map((c) => c.getAttribute('aria-label'))).toEqual(['Path 1 from a GC root, 2 steps', 'Path 2 from a GC root, 3 steps']);
    });

    it('shows instance and path errors with Retry', async () => {
      const { fixture } = await mount({ options: { fail: { heapInstances: 1, heapPaths: 1 } } });
      await showType(fixture, 'Shop.Orders.Customer');
      expect(el(fixture, '[role=alert]').textContent).toContain('heapInstances failed');
      buttonByText(fixture, 'Retry').click();
      await settle(fixture, 6);
      expect(instanceRows(fixture)).toHaveLength(30);
      at(instanceRows(fixture), 0).querySelector<HTMLButtonElement>('button')!.click();
      await settle(fixture, 6);
      expect(el(fixture, '.paths [role=alert]').textContent).toContain('heapPaths failed');
      el<HTMLButtonElement>(fixture, '.paths [role=alert] button').click();
      await settle(fixture, 6);
      expect(all(fixture, '[data-testid=root-path]').length).toBeGreaterThan(0);
    });

    it('lists the instances of a clicked type without requesting any paths', async () => {
      const { fixture, spies } = await mount();
      at(typeRows(fixture), 0).querySelector<HTMLButtonElement>('button.link')!.click();
      await settle(fixture, 6);
      expect(instanceRows(fixture).length).toBeGreaterThan(0);
      expect(spies.paths).not.toHaveBeenCalled();
    });

    it('shows the empty state for a type with no rows', async () => {
      const { fixture, backend } = await mount();
      vi.spyOn(backend, 'heapInstances').mockResolvedValue({ rows: [], totalRows: 0, totalBytes: 0 } satisfies HeapObjectsPage);
      await showType(fixture, 'X');
      expect(text(fixture)).toContain('No instances of this type.');
    });
  });
});
