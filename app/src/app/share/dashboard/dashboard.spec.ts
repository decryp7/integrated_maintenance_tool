import { TestBed } from '@angular/core/testing';
import { FakeECharts } from '../../../testing/fake-echarts';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend, typeInto } from '../../../testing/helpers';
import { AnalysisBackend } from '../../core/analysis-backend';
import { TileCfg, TileData, TileRequest } from '../../core/api.types';
import { DebriefBackend } from '../../core/backend';
import { MockBackend } from '../../core/mock/mock-backend';
import { WorkspaceStore } from '../../core/workspace.store';
import { mockJobs } from '../../jobs/mock-jobs';
import { MockAnalysisBackend, recordsFrom } from '../../analysis/mock/mock-analysis-backend';
import { CommandContext } from '../../extensions/tokens';
import { DialogService, DIALOG_REF } from '../../ui/dialog.service';
import { MockShareBackend } from '../mock/mock-share-backend';
import { ShareBackend } from '../share-backend';
import { AddTileDialog, TileForm, tileFromForm } from './add-tile-dialog';
import { DashboardCommands } from './dashboard.commands';
import { DashboardStore, toProfileDashboard } from './dashboard.store';
import { DASHBOARD_VIEWS } from './dashboard.views';
import { addTile, bottomOf, clampTile, moveTile, overlaps, placeTile, removeTile, resizeTile, resolveCollisions, sizeTile, uniqueId } from './dashboard-layout';
import { cellText, DashboardTile, sparkPoints } from './dashboard-tile';
import { DashboardView, tileKeyAction } from './dashboard-view';
import { at } from '../../../testing/must';

/** A profile save re-imports as a job: waits for it to finish and for the view to follow. */
async function applied(fixture?: Parameters<typeof settle>[0]): Promise<void> {
  await vi.waitFor(() => expect(TestBed.inject(WorkspaceStore).busy()).toBeNull(), { timeout: 5000 });
  await settle(fixture);
}

const charts = new FakeECharts();

const tile = (id: string, over: Partial<TileCfg> = {}): TileCfg => ({ id, title: id.toUpperCase(), type: 'annotations', x: 0, y: 0, w: 4, h: 3, ...over });

describe('dashboard layout', () => {
  it('clamps tiles to the grid', () => {
    expect(clampTile({ x: -2, y: -1, w: 0, h: 99 }, 12)).toEqual({ x: 0, y: 0, w: 1, h: 48 });
    expect(clampTile({ x: 11, y: 2.4, w: 4, h: 2 }, 12)).toEqual({ x: 8, y: 2, w: 4, h: 2 });
    expect(clampTile({ x: 0, y: 0, w: 30, h: 1 }, 12).w).toBe(12);
    expect(clampTile({ x: 3, y: 0, w: 2, h: 1 }, 0)).toEqual({ x: 0, y: 0, w: 1, h: 1 });
  });

  it('detects overlaps and the bottom row', () => {
    expect(overlaps({ x: 0, y: 0, w: 2, h: 2 }, { x: 1, y: 1, w: 2, h: 2 })).toBe(true);
    expect(overlaps({ x: 0, y: 0, w: 2, h: 2 }, { x: 2, y: 0, w: 2, h: 2 })).toBe(false);
    expect(overlaps({ x: 0, y: 0, w: 2, h: 2 }, { x: 0, y: 2, w: 2, h: 2 })).toBe(false);
    expect(bottomOf([])).toBe(0);
    expect(bottomOf([tile('a', { y: 2, h: 3 }), tile('b', { h: 1 })])).toBe(5);
  });

  it('pushes overlapped tiles down, in order, keeping list order', () => {
    const tiles = [tile('a', { x: 0, y: 3 }), tile('b', { x: 0, y: 0 }), tile('c', { x: 0, y: 6 })];
    const out = resolveCollisions(tiles, 'b');
    expect(out.map((t) => t.id)).toEqual(['a', 'b', 'c']);
    expect(out.map((t) => t.y)).toEqual([3, 0, 6]);
    const pushed = resolveCollisions([tile('a', { y: 0 }), tile('b', { y: 1 }), tile('c', { y: 3 })], 'b');
    expect(pushed.find((t) => t.id === 'a')!.y).toBe(4);
    expect(pushed.find((t) => t.id === 'c')!.y).toBe(7);
    expect(resolveCollisions(tiles, 'zz')).toEqual(tiles);
  });

  it('moves, places, resizes, sizes, removes and adds tiles', () => {
    const tiles = [tile('a'), tile('b', { x: 4 })];
    expect(moveTile(tiles, 'a', 1, 0, 12)[0]).toMatchObject({ x: 1, y: 0 });
    expect(moveTile(tiles, 'a', 1, 0, 12)[1]).toMatchObject({ x: 4, y: 3 });
    expect(moveTile(tiles, 'a', -5, -5, 12)[0]).toMatchObject({ x: 0, y: 0 });
    expect(moveTile(tiles, 'a', 20, 0, 12)[0]?.x).toBe(8);
    expect(moveTile(tiles, 'zz', 1, 1, 12)).toEqual(tiles);
    expect(placeTile(tiles, 'b', 0, 0, 12).find((t) => t.id === 'a')!.y).toBe(3);
    expect(resizeTile(tiles, 'b', 1, 1, 12)[1]).toMatchObject({ w: 5, h: 4 });
    expect(resizeTile(tiles, 'b', 20, -5, 12)[1]).toMatchObject({ w: 8, h: 1 });
    expect(resizeTile(tiles, 'a', -9, 0, 12)[0]?.w).toBe(1);
    expect(sizeTile(tiles, 'b', 50, 2, 12)[1]).toMatchObject({ w: 8, h: 2 });
    expect(sizeTile(tiles, 'b', 0, 2, 12)[1]?.w).toBe(1);
    expect(removeTile(tiles, 'a').map((t) => t.id)).toEqual(['b']);
    expect(addTile(tiles, tile('c', { x: 9, w: 20 }), 12)[2]).toMatchObject({ x: 0, y: 3, w: 12 });
  });

  it('makes unique ids', () => {
    expect(uniqueId([], 'My Tile!')).toBe('my-tile');
    expect(uniqueId(['my-tile', 'my-tile-2'], 'My tile')).toBe('my-tile-3');
    expect(uniqueId([], '***')).toBe('tile');
  });
});

describe('tile helpers', () => {
  it('draws sparklines and formats cells', () => {
    expect(sparkPoints([])).toBe('');
    expect(sparkPoints([[0, 5]])).toBe('');
    expect(sparkPoints([[0, 0], [1, null], [2, 10]])).toBe('0.0,22.0 100.0,2.0');
    expect(cellText('@t', 0, 'utc')).toMatch(/00:00:00/);
    expect(cellText('x', null, 'utc')).toBe('');
    expect(cellText('x', 1234.5, 'utc')).toMatch(/1/);
    expect(cellText('x', 'a', 'utc')).toBe('a');
    expect(cellText('x', { a: 1 }, 'utc')).toBe('{"a":1}');
  });

  it('maps keys to tile actions', () => {
    expect(tileKeyAction('ArrowRight', false)).toEqual({ kind: 'move', dx: 1, dy: 0 });
    expect(tileKeyAction('ArrowUp', true)).toEqual({ kind: 'resize', dx: 0, dy: -1 });
    expect(tileKeyAction('Delete', false)).toEqual({ kind: 'remove' });
    expect(tileKeyAction('Backspace', true)).toEqual({ kind: 'remove' });
    expect(tileKeyAction('a', false)).toBeNull();
  });
});

describe('add tile form', () => {
  const form = (over: Partial<TileForm>): TileForm => ({ type: 'stat', title: '', chart: '', value: '', by: '', style: 'line', query: '', unit: '', field: '', limit: '', window: '', ...over });

  it('builds tiles of every kind', () => {
    expect(tileFromForm(form({ type: 'chart', chart: 'cpu', query: 'x = 1', title: 'CPU' }))).toEqual({ tile: { id: 'CPU', title: 'CPU', type: 'chart', x: 0, y: 0, w: 6, h: 4, chart: 'cpu', query: 'x = 1' } });
    expect(tileFromForm(form({ type: 'chart', value: 'count()', by: '@level, Machine', style: 'bar' }))).toMatchObject({ tile: { value: 'count()', by: ['@level', 'Machine'], style: 'bar' } });
    expect(tileFromForm(form({ type: 'chart', value: 'count()' }))).not.toHaveProperty('tile.by');
    expect(tileFromForm(form({ query: '| stats count()', unit: 'ms', window: '15m' }))).toMatchObject({ tile: { query: '| stats count()', unit: 'ms', window: '15m', w: 3, h: 2 } });
    expect(tileFromForm(form({ query: '| stats count()' }))).not.toHaveProperty('tile.unit');
    expect(tileFromForm(form({ type: 'table', query: 'a', limit: '5' }))).toMatchObject({ tile: { query: 'a', limit: 5 } });
    expect(tileFromForm(form({ type: 'table', query: 'a' }))).not.toHaveProperty('tile.limit');
    expect(tileFromForm(form({ type: 'top', field: 'F', query: 'q', limit: '3' }))).toMatchObject({ tile: { field: 'F', query: 'q', limit: 3 } });
    expect(tileFromForm(form({ type: 'top', field: 'F' }))).not.toHaveProperty('tile.query');
    expect(tileFromForm(form({ type: 'alerts', query: 'q', limit: '2' }))).toMatchObject({ tile: { query: 'q', limit: 2 } });
    expect(tileFromForm(form({ type: 'alerts' }))).not.toHaveProperty('tile.query');
    expect(tileFromForm(form({ type: 'annotations', limit: '4' }))).toMatchObject({ tile: { limit: 4 } });
    expect(tileFromForm(form({ type: 'annotations' }))).not.toHaveProperty('tile.limit');
  });

  it('says what is missing', () => {
    expect(tileFromForm(form({ type: 'chart' }))).toEqual({ error: expect.stringMatching(/Choose a chart/) });
    expect(tileFromForm(form({ query: 'level = error' }))).toEqual({ error: expect.stringMatching(/stats query/) });
    expect(tileFromForm(form({ type: 'table' }))).toEqual({ error: expect.stringMatching(/needs a query/) });
    expect(tileFromForm(form({ type: 'top' }))).toEqual({ error: expect.stringMatching(/Choose a field/) });
    expect(tileFromForm(form({ type: 'table', query: 'a', limit: 'x' }))).toEqual({ error: expect.stringMatching(/Limit/) });
    expect(tileFromForm(form({ type: 'table', query: 'a', limit: '0' }))).toEqual({ error: expect.stringMatching(/Limit/) });
  });
});

function provideShareTesting(backend: MockBackend = tinyBackend()) {
  return [
    provideTinyBackend(backend),
    charts.provider,
    { provide: AnalysisBackend, useFactory: () => new MockAnalysisBackend(recordsFrom(TestBed.inject(DebriefBackend))) },
    { provide: ShareBackend, useFactory: () => new MockShareBackend(TestBed.inject(DebriefBackend), TestBed.inject(AnalysisBackend)) },
  ];
}

async function setup(backend?: MockBackend) {
  TestBed.configureTestingModule({ providers: provideShareTesting(backend) });
  const store = TestBed.inject(WorkspaceStore);
  await settle();
  return { store, ds: TestBed.inject(DashboardStore) };
}

describe('DashboardStore', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('lists the profile dashboards and selects one', async () => {
    const { ds } = await setup();
    expect(ds.dashboards().map((d) => d.title)).toEqual(['OrderService health']);
    expect(ds.current()?.id).toBe('orderservice-health');
    ds.select('nope');
    expect(ds.current()?.id).toBe('orderservice-health');
  });

  it('edits a draft, saves it to the profile and cancels', async () => {
    const { ds, store } = await setup();
    ds.startEdit();
    ds.startEdit();
    expect(ds.editing()).toBe(true);
    ds.select('other');
    expect(ds.selectedId()).toBeNull();
    const first = at(ds.current()!.tile, 0).id;
    ds.move(first, 1, 0);
    expect(ds.current()!.tile[0]?.x).toBe(1);
    expect(ds.announcement()).toMatch(/column 2, row 1/);
    ds.resize(first, 1, 1);
    expect(ds.current()!.tile[0]).toMatchObject({ w: 4, h: 3 });
    ds.place(first, 0, 0);
    ds.setSize(first, 3, 2);
    ds.setTitle('Health');
    const id = ds.add(tile('Extra Tile', { type: 'annotations' }))!;
    expect(id).toBe('extra-tile');
    expect(ds.add(tile('extra-tile'))).toBe('extra-tile-2');
    ds.remove('extra-tile-2');
    expect(ds.announcement()).toMatch(/removed/);
    expect(await ds.save()).toBe(true);
    await applied();
    expect(ds.editing()).toBe(false);
    const saved = at(store.summary()!.dashboards, 0);
    expect(saved.title).toBe('Health');
    expect(saved.tile.some((t) => t.id === 'extra-tile')).toBe(true);

    ds.startEdit();
    ds.cancel();
    expect(ds.editing()).toBe(false);
    expect(await ds.save()).toBe(false);
    expect(ds.add(tile('x'))).toBeNull();
    ds.move('x', 1, 1);
  });

  it('creates dashboards and shows the engine refusal', async () => {
    const { ds, store } = await setup();
    ds.newDashboard('Ops wall');
    expect(ds.current()).toMatchObject({ id: 'ops-wall', columns: 12, tile: [] });
    ds.add({ ...tile('bad'), type: 'stat' });
    expect(await ds.save()).toBe(false);
    expect(ds.error()).toMatch(/stat tile needs a query/);
    expect(ds.editing()).toBe(true);
    ds.remove('bad');
    expect(await ds.save()).toBe(true);
    await applied();
    expect(store.summary()!.dashboards.map((d) => d.id)).toEqual(['orderservice-health', 'ops-wall']);
    expect(ds.current()?.id).toBe('ops-wall');
    ds.startEdit();
    ds.setTitle('Ops');
    await ds.save();
    await applied();
    expect(store.summary()!.dashboards.map((d) => d.title)).toEqual(['OrderService health', 'Ops']);
  });

  it('serialises only the keys that are set', () => {
    expect(toProfileDashboard({ id: 'd', title: 'D', columns: 12, tile: [{ ...tile('t'), title: '', query: '', by: [], unit: undefined }] })).toEqual({
      id: 'd',
      title: 'D',
      columns: 12,
      tile: [{ id: 't', title: '', type: 'annotations', x: 0, y: 0, w: 4, h: 3 }],
    });
  });
});

/** A share backend whose tile answers the test controls. */
class ScriptedShare extends MockShareBackend {
  answers: (TileData | Error)[] = [];
  requests: TileRequest[] = [];
  override async dashboardTile(request: TileRequest): Promise<TileData> {
    this.requests.push(request);
    const next = this.answers.shift();
    if (next instanceof Error) throw next;
    return next ?? super.dashboardTile(request);
  }
}

describe('DashboardTile', () => {
  afterEach(() => TestBed.resetTestingModule());

  async function render(t: TileCfg, answers: (TileData | Error)[] = []) {
    const share = new ScriptedShare(tinyBackend(), new MockAnalysisBackend(async () => []));
    share.answers = answers;
    TestBed.configureTestingModule({ providers: [...provideShareTesting(), { provide: ShareBackend, useValue: share }] });
    const fixture = TestBed.createComponent(DashboardTile);
    fixture.componentRef.setInput('tile', t);
    await settle(fixture);
    return { fixture, share };
  }

  it('renders a stat with unit and sparkline', async () => {
    const { fixture } = await render(tile('s', { type: 'stat' }), [{ kind: 'stat', value: 1234, text: '1234', label: 'errors', unit: 'ms', sparkline: [[0, 1], [1, 3]] }]);
    expect(el(fixture, '[data-testid=stat-value]').textContent).toMatch(/1,?234\s*ms/);
    expect(text(fixture)).toContain('errors');
    expect(all(fixture, 'polyline')).toHaveLength(1);
  });

  it('shows text values and a dash for empty stats', async () => {
    const a = await render(tile('s'), [{ kind: 'stat', value: null, text: 'n/a', label: 'x', unit: null, sparkline: [] }]);
    expect(el(a.fixture, '[data-testid=stat-value]').textContent).toBe('n/a');
    expect(all(a.fixture, 'polyline')).toHaveLength(0);
    TestBed.resetTestingModule();
    const b = await render(tile('s'), [{ kind: 'stat', value: null, text: '', label: 'x', unit: null, sparkline: [] }]);
    expect(el(b.fixture, '[data-testid=stat-value]').textContent).toBe('—');
  });

  it('renders tables, top values, alerts and annotations', async () => {
    const table = await render(tile('t'), [{ kind: 'table', columns: ['@t', 'n'], rows: [[0, 5]], truncated: true }]);
    expect(all(table.fixture, 'th').map((x) => x.textContent)).toEqual(['@t', 'n']);
    expect(text(table.fixture)).toContain('More rows not shown');
    TestBed.resetTestingModule();
    const empty = await render(tile('t'), [{ kind: 'table', columns: ['a'], rows: [], truncated: false }]);
    expect(text(empty.fixture)).toContain('No rows');
    TestBed.resetTestingModule();
    const top = await render(tile('t'), [{ kind: 'top', facet: { field: 'f', values: [{ value: 'A', count: 10 }, { value: 'B', count: 5 }], distinct: 2, total: 15 } }]);
    expect(all<HTMLElement>(top.fixture, '.bar').map((b) => b.style.width)).toEqual(['100%', '50%']);
    TestBed.resetTestingModule();
    const noTop = await render(tile('t'), [{ kind: 'top', facet: { field: 'f', values: [], distinct: 0, total: 0 } }]);
    expect(text(noTop.fixture)).toContain('No values');
    TestBed.resetTestingModule();
    const alerts = await render(tile('t'), [{ kind: 'alerts', items: [{ kind: 'errorSpike', instance: 'WEB01', from: 0, to: 1, score: 3, message: 'Errors up', sampleIds: [] }, { kind: 'silence', instance: null, from: 0, to: 1, score: 1, message: 'Quiet', sampleIds: [] }] } as TileData]);
    expect(text(alerts.fixture)).toContain('Errors up');
    expect(text(alerts.fixture)).toContain('WEB01');
    const meta = el(alerts.fixture, '.meta');
    expect(meta.children[0]?.textContent).toBe('errorSpike');
    expect(getComputedStyle(meta).display).toBe('flex');
    TestBed.resetTestingModule();
    const noAlerts = await render(tile('t'), [{ kind: 'alerts', items: [] }]);
    expect(text(noAlerts.fixture)).toContain('No alerts');
    TestBed.resetTestingModule();
    const ann = { annotator: 'a', owner: 'o', kind: 'k', instance: null, on: null, severity: 'info' as const, evidenceQuery: {} as never };
    const notes = await render(tile('t'), [{ kind: 'annotations', items: [{ ...ann, from: 0, to: 0, label: 'Reboot' }, { ...ann, from: 0, to: 60_000, label: 'Up' }] }]);
    expect(text(notes.fixture)).toContain('Reboot');
    expect(text(notes.fixture)).toContain('–');
    TestBed.resetTestingModule();
    const none = await render(tile('t'), [{ kind: 'annotations', items: [] }]);
    expect(text(none.fixture)).toContain('No annotations');
  });

  it('renders charts through ECharts, or says when empty', async () => {
    charts.reset();
    const { fixture } = await render(tile('c', { type: 'chart', chart: 'events-by-level' }));
    expect(charts.instances.length).toBe(1);
    expect(fixture.nativeElement.querySelector('debrief-echart')).toBeTruthy();
    TestBed.resetTestingModule();
    const def = { id: 'c', title: 'C', y: 'count()', bucket: 'auto', splitBy: [], type: 'line' as const, unit: '' };
    const empty = await render(tile('c'), [{ kind: 'chart', data: { def, from: 0, to: 1, bucketMs: 1, series: [], lifecycle: [] } }]);
    expect(text(empty.fixture)).toContain('No data in this range');
  });

  it('shows an inline error with Retry, and refetches when the query changes', async () => {
    const { fixture, share } = await render(tile('t', { type: 'stat', query: '| stats count()' }), [new Error('tile broke')]);
    expect(el(fixture, '[role=alert]').textContent).toContain('tile broke');
    share.answers.push('oops' as never as Error);
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(share.requests.length).toBe(2);
    TestBed.inject(WorkspaceStore).setRange(1, 2);
    await settle(fixture);
    expect(share.requests.length).toBe(3);
    expect(share.requests[2]?.query.from).toBe(1);
    expect(el(fixture, '[data-testid=stat-value]')).toBeTruthy();
  });
});

describe('AddTileDialog', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('builds a tile from the form and closes with it', async () => {
    const closed: unknown[] = [];
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), { provide: DIALOG_REF, useValue: { close: (r: unknown) => closed.push(r), closed: Promise.resolve() } }] });
    await settle();
    const fixture = TestBed.createComponent(AddTileDialog);
    await settle(fixture);
    buttonByText(fixture, 'Add tile').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toMatch(/stats query/);
    typeInto(el(fixture, 'input[name=title]'), 'Errors');
    typeInto(el(fixture, 'input[name=query]'), 'level >= error | stats count()');
    typeInto(el(fixture, 'input[name=unit]'), 'ev');
    typeInto(el(fixture, 'input[name=window]'), '5m');
    buttonByText(fixture, 'Add tile').click();
    expect(closed[0]).toMatchObject({ title: 'Errors', type: 'stat', unit: 'ev', window: '5m' });

    for (const kind of ['chart', 'table', 'top', 'alerts', 'annotations']) {
      typeInto(el(fixture, 'select[name=type]'), kind, 'change');
      await settle(fixture);
    }
    typeInto(el(fixture, 'select[name=type]'), 'chart', 'change');
    await settle(fixture);
    expect(all(fixture, 'select[name=chart] option').length).toBeGreaterThan(1);
    typeInto(el(fixture, 'input[name=value]'), 'count()');
    typeInto(el(fixture, 'input[name=by]'), '@level');
    typeInto(el(fixture, 'select[name=style]'), 'bar', 'change');
    typeInto(el(fixture, 'select[name=chart]'), 'cpu', 'change');
    await settle(fixture);
    expect(all(fixture, 'input[name=value]')).toHaveLength(0);
    buttonByText(fixture, 'Add tile').click();
    expect(closed[1]).toMatchObject({ type: 'chart', chart: 'cpu' });
    typeInto(el(fixture, 'select[name=type]'), 'top', 'change');
    await settle(fixture);
    typeInto(el(fixture, 'select[name=field]'), 'SourceContext', 'change');
    typeInto(el(fixture, 'input[name=limit]'), '4');
    buttonByText(fixture, 'Add tile').click();
    expect(closed[2]).toMatchObject({ type: 'top', field: 'SourceContext', limit: 4 });
    typeInto(el(fixture, 'select[name=type]'), 'table', 'change');
    await settle(fixture);
    typeInto(el(fixture, 'input[name=query]'), '| stats count() by SourceContext');
    buttonByText(fixture, 'Cancel').click();
    expect(closed[3]).toBeUndefined();
  });
});

describe('DashboardView', () => {
  afterEach(() => TestBed.resetTestingModule());

  async function render() {
    await setup();
    const fixture = TestBed.createComponent(DashboardView);
    await settle(fixture);
    return { fixture, ds: TestBed.inject(DashboardStore) };
  }

  it('lays the tiles out on the grid', async () => {
    const { fixture } = await render();
    const tiles = all<HTMLElement>(fixture, '.tile');
    expect(tiles.length).toBe(8);
    expect(tiles[0]?.style.gridColumn).toMatch(/1 \/ span 3/);
    expect(tiles[0]?.getAttribute('data-size')).toBe('3x2');
    expect(tiles[0]?.getAttribute('tabindex')).toBeNull();
    expect(text(fixture)).toContain('Errors');
  });

  it('moves, resizes and removes tiles with the keyboard in edit mode', async () => {
    const { fixture, ds } = await render();
    buttonByText(fixture, 'Edit').click();
    await settle(fixture);
    const first = () => el<HTMLElement>(fixture, '[data-tile=errors]');
    first().dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', shiftKey: true, bubbles: true }));
    await settle(fixture);
    expect(first().getAttribute('data-size')).toBe('4x2');
    first().dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    await settle(fixture);
    expect(first().getAttribute('data-pos')).toBe('0,1');
    expect(el(fixture, '[data-testid=dashboard-announce]').textContent).toMatch(/row 2/);
    first().dispatchEvent(new KeyboardEvent('keydown', { key: 'x', bubbles: true }));
    first().dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete', bubbles: true }));
    await settle(fixture);
    expect(all(fixture, '[data-tile=errors]')).toHaveLength(0);
    el<HTMLButtonElement>(fixture, '[aria-label="Remove tile Warnings"]').click();
    await settle(fixture);
    expect(ds.current()!.tile.some((t) => t.id === 'warnings')).toBe(false);
    typeInto(el(fixture, '.title-edit input'), 'Renamed');
    buttonByText(fixture, 'Cancel').click();
    await settle(fixture);
    expect(all(fixture, '.tile').length).toBe(8);
  });

  it('drags and resizes with the pointer', async () => {
    const { fixture, ds } = await render();
    buttonByText(fixture, 'Edit').click();
    await settle(fixture);
    const grid = el<HTMLElement>(fixture, '.grid');
    Object.defineProperty(grid, 'clientWidth', { value: 1192 });
    const tileEl = el<HTMLElement>(fixture, '[data-tile=errors]');
    const down = (target: Element, x: number, y: number) => target.dispatchEvent(Object.assign(new MouseEvent('pointerdown', { clientX: x, clientY: y, bubbles: true, button: 0 }), {}));
    down(tileEl, 10, 10);
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 210, clientY: 82 }));
    window.dispatchEvent(new MouseEvent('pointerup'));
    await settle(fixture);
    expect(ds.current()!.tile.find((t) => t.id === 'errors')).toMatchObject({ x: 2, y: 1 });
    down(el(fixture, '[data-tile=errors] .resizer'), 0, 0);
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 100, clientY: 72 }));
    window.dispatchEvent(new MouseEvent('pointerup'));
    window.dispatchEvent(new MouseEvent('pointermove', { clientX: 900, clientY: 900 }));
    await settle(fixture);
    expect(ds.current()!.tile.find((t) => t.id === 'errors')).toMatchObject({ w: 4, h: 3 });
    // A press on a button inside the tile does not start a drag.
    down(el(fixture, '[data-tile=errors] .remove'), 0, 0);
  });

  it('adds tiles through the dialog and saves', async () => {
    const { fixture, ds } = await render();
    const dialogs = TestBed.inject(DialogService);
    buttonByText(fixture, 'Edit').click();
    await settle(fixture);
    buttonByText(fixture, /Add tile/).click();
    await TestBed.inject(DialogService).whenLoaded();
    await settle(fixture);
    at(dialogs.stack(), 0).ref.close({ ...tile('new one'), type: 'annotations' });
    await settle(fixture);
    expect(ds.current()!.tile.some((t) => t.id === 'new-one')).toBe(true);
    buttonByText(fixture, /Add tile/).click();
    await settle(fixture);
    at(dialogs.stack(), 0).ref.close();
    buttonByText(fixture, 'Save').click();
    await settle(fixture);
    expect(ds.editing()).toBe(false);
  });

  it('enters and leaves wall mode and offers a first dashboard', async () => {
    const { fixture, ds } = await render();
    buttonByText(fixture, 'Wall mode').click();
    await settle(fixture);
    expect(fixture.nativeElement.classList.contains('wall')).toBe(true);
    expect(text(fixture)).toContain('OrderService health');
    fixture.nativeElement.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await settle(fixture);
    expect(ds.wall()).toBe(false);
    buttonByText(fixture, 'Wall mode').click();
    await settle(fixture);
    buttonByText(fixture, 'Exit wall mode').click();
    expect(ds.wall()).toBe(false);
    // A full-screen request the platform drops keeps the full-viewport wall mode...
    ds.wall.set(true);
    document.dispatchEvent(new Event('fullscreenchange'));
    expect(ds.wall()).toBe(true);
    // ...leaving real full screen (Escape) leaves wall mode.
    let full: Element | null = fixture.nativeElement;
    Object.defineProperty(document, 'fullscreenElement', { configurable: true, get: () => full });
    document.dispatchEvent(new Event('fullscreenchange'));
    full = null;
    document.dispatchEvent(new Event('fullscreenchange'));
    expect(ds.wall()).toBe(false);
    delete (document as unknown as Record<string, unknown>)['fullscreenElement'];
    buttonByText(fixture, /New dashboard/).click();
    await settle(fixture);
    expect(text(fixture)).toContain('no tiles yet');
    expect(text(fixture)).toContain('Add tile');
  });

  it('explains when the profile has no dashboards', async () => {
    const backend = tinyBackend();
    await mockJobs().wait(await backend.saveProfile('name = "x"'));
    await setup(backend);
    const fixture = TestBed.createComponent(DashboardView);
    await settle(fixture);
    expect(text(fixture)).toContain('No dashboards in this profile yet');
  });
});

describe('dashboard commands and view', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('registers the view and runs its commands', async () => {
    expect(DASHBOARD_VIEWS[0]?.id).toBe('dashboards');
    expect(await DASHBOARD_VIEWS[0]?.component()).toBe(DashboardView);
    const { ds } = await setup();
    const shown: string[] = [];
    const ctx = { showView: (id: string) => shown.push(id) } as unknown as CommandContext;
    const cmds = TestBed.inject(DashboardCommands).commands;
    const cmd = (id: string) => cmds.find((c) => c.id === id)!;
    await cmd('dashboards.show').run(ctx);
    expect(cmd('dashboards.addTile').when!(ctx)).toBe(false);
    expect(cmd('dashboards.edit').when!(ctx)).toBe(true);
    await cmd('dashboards.edit').run(ctx);
    expect(ds.editing()).toBe(true);
    expect(cmd('dashboards.addTile').when!(ctx)).toBe(true);
    const done = cmd('dashboards.addTile').run(ctx);
    await settle();
    at(TestBed.inject(DialogService).stack(), 0).ref.close(tile('cmd'));
    await done;
    expect(ds.current()!.tile.some((t) => t.id === 'cmd')).toBe(true);
    const nothing = cmd('dashboards.addTile').run(ctx);
    await settle();
    at(TestBed.inject(DialogService).stack(), 0).ref.close();
    await nothing;
    await cmd('dashboards.edit').run(ctx);
    expect(ds.editing()).toBe(false);
    expect(cmd('dashboards.wall').when!(ctx)).toBe(true);
    await cmd('dashboards.wall').run(ctx);
    expect(ds.wall()).toBe(true);
    expect(shown).toEqual(['dashboards', 'dashboards', 'dashboards', 'dashboards']);
  });
});
