import { TestBed } from '@angular/core/testing';
import { all, el, FakeDock, provideFakeDock, provideTinyBackend, settle, typeInto } from '../../../testing/helpers';
import { RecordView } from '../../core/api.types';
import { DebriefBackend } from '../../core/backend';
import { MockBackend } from '../../core/mock/mock-backend';
import { MockDataset, registryRecords } from '../../core/mock/mock-data';
import { WorkspaceStore } from '../../core/workspace.store';
import { loadRegistryRows } from './registry-loader';
import { REGISTRY_PLUGIN } from './registry.plugin';
import { buildRegistryTree, dataText, expandableIds, flattenTree, REGISTRY_ROW_CAP } from './registry-tree';
import { RegistryView } from './registry-view';
import { tinyDataset } from '../../../testing/helpers';
import { at } from '../../../testing/must';

const row = (fields: Record<string, unknown>, id = 0): RecordView =>
  ({ id, ts: null, tsState: 'untimed', level: 'Information', message: '', template: null, exception: null, fields, source: 5, sourceName: 'r.reg', line: id, byteOffset: 0, group: '', markers: [] }) as RecordView;

const sample = (): RecordView[] => [
  row({ Hive: 'HKLM', Key: 'SOFTWARE\\Contoso', ValueType: 'KEY' }, 1),
  row({ Hive: 'HKLM', Key: 'SOFTWARE\\Contoso', ValueName: 'Version', ValueType: 'REG_SZ', Data: '1.2' }, 2),
  row({ Hive: 'HKLM', Key: 'SOFTWARE\\Contoso', ValueName: 'Version', ValueType: 'REG_SZ', Data: 'dup' }, 3),
  row({ Hive: 'HKLM', Key: 'SOFTWARE\\Contoso\\Sub', ValueName: 'Timeout', ValueType: 'REG_DWORD', Data: 30 }, 4),
  row({ Hive: 'HKLM', Key: 'SOFTWARE\\Contoso\\Sub', ValueName: 'Sinks', ValueType: 'REG_MULTI_SZ', Data: ['File', 'Console'] }, 5),
  row({ Hive: 'HKLM', Key: 'SOFTWARE\\Contoso\\Gone', ValueType: 'DELETE' }, 6),
  row({ Hive: 'HKCU', Key: 'Software\\X', ValueName: '(Default)', ValueType: 'REG_SZ', Data: 'x' }, 7),
  row({ Key: 'no-hive' }, 8),
  row({ Hive: 'HKLM', Key: '', ValueName: 'AtRoot', ValueType: 'REG_SZ', Data: null }, 9),
];

describe('registry tree', () => {
  it('builds hive -> key segments -> values, sorted keys first, ignoring rows without a hive and duplicate values', () => {
    const tree = buildRegistryTree(sample());
    expect(tree.map((n) => n.label)).toEqual(['HKCU', 'HKLM']);
    const hklm = at(tree, 1);
    expect(hklm.children.map((n) => `${n.kind}:${n.label}`)).toEqual(['key:SOFTWARE', 'value:AtRoot']);
    const contoso = at(at(hklm.children, 0).children, 0);
    expect(contoso.path).toBe('HKLM\\SOFTWARE\\Contoso');
    expect(contoso.record?.id).toBe(1);
    expect(contoso.children.map((n) => `${n.kind}:${n.label}`)).toEqual(['key:Gone', 'key:Sub', 'value:Version']);
    expect(at(contoso.children, 0).deleted).toBe(true);
    const version = at(contoso.children, 2);
    expect(version).toMatchObject({ valueType: 'REG_SZ', data: '1.2', path: 'HKLM\\SOFTWARE\\Contoso\\Version' });
    expect(version.record?.id).toBe(2);
    const sub = at(contoso.children, 1);
    expect(sub.children.map((n) => n.data)).toEqual(['File | Console', '30']);
    expect(at(hklm.children, 1).data).toBe('');
  });

  it('renders data on one line', () => {
    expect(dataText('a\nb')).toBe('a | b');
    expect(dataText(['x', 'y'])).toBe('x | y');
    expect(dataText(7)).toBe('7');
    expect(dataText(null)).toBe('');
    expect(dataText(undefined)).toBe('');
  });

  it('flattens only expanded nodes with ARIA metadata', () => {
    const tree = buildRegistryTree(sample());
    expect(flattenTree(tree, new Set()).map((f) => f.node.label)).toEqual(['HKCU', 'HKLM']);
    const ids = expandableIds(tree);
    const flat = flattenTree(tree, new Set(ids));
    const version = flat.find((f) => f.node.label === 'Version')!;
    expect(version).toMatchObject({ level: 4, posInSet: 3, setSize: 3, expandable: false, expanded: false });
    expect(flat[0]).toMatchObject({ level: 1, posInSet: 1, setSize: 2, expandable: true, expanded: true });
    expect(flattenTree(tree, new Set(['HKLM'])).map((f) => f.node.label)).toEqual(['HKCU', 'HKLM', 'SOFTWARE', 'AtRoot']);
  });

  it('filters by name, type or data, keeping and expanding ancestors', () => {
    const tree = buildRegistryTree(sample());
    const labels = (q: string) => flattenTree(tree, new Set(), q).map((f) => f.node.label);
    expect(labels('timeout')).toEqual(['HKLM', 'SOFTWARE', 'Contoso', 'Sub', 'Timeout']);
    expect(labels('reg_dword')).toEqual(['HKLM', 'SOFTWARE', 'Contoso', 'Sub', 'Timeout']);
    expect(labels('1.2')).toEqual(['HKLM', 'SOFTWARE', 'Contoso', 'Version']);
    expect(labels('  ')).toEqual(['HKCU', 'HKLM']);
    expect(labels('zzz')).toEqual([]);
  });
});

describe('loadRegistryRows', () => {
  const source = (id: number, name: string) => ({ id, name, path: `/${name}`, group: `/${name}`, importer: 'registry', fileKind: 'text' as const, records: 0, skipped: 0, dropped: 0, error: null, from: null, to: null });

  it('pages through every registry source filtered by @source', async () => {
    const pages: number[] = [];
    const backend = {
      queryPage: async (req: { offset: number; limit: number; query: { filters: { value: string }[] } }) => {
        pages.push(req.offset);
        const all = Array.from({ length: 5 }, (_, i) => ({ ...row({ Hive: 'H', Key: `K${i}` }, i), source: at(req.query.filters, 0).value === 'a.reg' ? 1 : 2 }));
        return { total: 5, offset: req.offset, anchorIndex: null, rows: all.slice(req.offset, req.offset + 2) };
      },
    } as unknown as DebriefBackend;
    const out = await loadRegistryRows(backend, [source(1, 'a.reg'), source(2, 'b.reg')], 100);
    expect(out.truncated).toBe(false);
    expect(out.rows).toHaveLength(10);
    expect(pages).toEqual([0, 2, 4, 0, 2, 4]);
  });

  it('caps the number of rows and says so', async () => {
    const backend = { queryPage: async (req: { limit: number }) => ({ total: 100, offset: 0, anchorIndex: null, rows: Array.from({ length: req.limit }, (_, i) => ({ ...row({ Hive: 'H', Key: `K${i}` }), source: 1 })) }) } as unknown as DebriefBackend;
    const out = await loadRegistryRows(backend, [source(1, 'a.reg'), source(2, 'b.reg')], 7);
    expect(out.rows).toHaveLength(7);
    expect(out.truncated).toBe(true);
    expect(REGISTRY_ROW_CAP).toBe(50_000);
  });

  it('stops on an empty page', async () => {
    const backend = { queryPage: async () => ({ total: 9, offset: 0, anchorIndex: null, rows: [] }) } as unknown as DebriefBackend;
    expect((await loadRegistryRows(backend, [source(1, 'a.reg')])).rows).toEqual([]);
  });
});

describe('registry plugin definition', () => {
  it('offers the view and command only when a registry source exists', () => {
    const view = at(REGISTRY_PLUGIN.views!, 0);
    const cmd = at(REGISTRY_PLUGIN.commands!, 0);
    const summary = (importer: string) => ({ sources: [{ importer }] }) as never;
    expect(view.when!(summary('registry'))).toBe(true);
    expect(view.when!(summary('json'))).toBe(false);
    expect(cmd.when!({ summary: summary('registry') } as never)).toBe(true);
    expect(cmd.when!({ summary: null } as never)).toBe(false);
    const showView = vi.fn();
    cmd.run({ showView } as never);
    expect(showView).toHaveBeenCalledWith('registry');
  });
});

describe('RegistryView', () => {
  function dataset(): MockDataset {
    const d = tinyDataset();
    d.sources.push({ id: 5, path: '/x/r.reg', name: 'r.reg', importer: 'registry', skipped: 0 });
    d.records.push(...registryRecords(5));
    return d;
  }

  async function render(backend = new MockBackend({ latency: false, dataset })) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend), provideFakeDock(new FakeDock())] });
    const fixture = TestBed.createComponent(RegistryView);
    await settle(fixture, 8);
    return { fixture, store: TestBed.inject(WorkspaceStore) };
  }

  const labels = (f: { nativeElement: HTMLElement }) => [...f.nativeElement.querySelectorAll('[role=treeitem] .label')].map((l) => l.textContent);
  const key = (f: { nativeElement: HTMLElement }, k: string) => f.nativeElement.querySelector('[role=tree]')!.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true }));

  it('shows the tree with type badges and data, selecting a value selects its record', async () => {
    const { fixture, store } = await render();
    expect(el(fixture, '[role=tree]').getAttribute('aria-label')).toBe('Registry');
    expect(labels(fixture)).toContain('HKEY_LOCAL_MACHINE');
    expect(labels(fixture)).toContain('TimeoutSeconds');
    const timeout = all(fixture, '[role=treeitem]').find((i) => i.textContent?.includes('TimeoutSeconds'))!;
    expect(timeout.textContent).toContain('REG_DWORD');
    expect(timeout.textContent).toContain('30');
    timeout.click();
    await settle(fixture);
    expect(store.selected()?.fields['ValueName']).toBe('TimeoutSeconds');
    expect(timeout.getAttribute('aria-selected')).toBe('true');
    expect(fixture.nativeElement.textContent).toContain('DELETED KEY');
  });

  it('expands and collapses with keys, moves with arrows and Home/End', async () => {
    const { fixture } = await render();
    const focus = () => el(fixture, '[role=treeitem][tabindex="0"]');
    expect(focus().textContent).toContain('HKEY_CURRENT_USER');
    key(fixture, 'ArrowLeft'); // collapse the expanded hive
    await settle(fixture);
    expect(labels(fixture)).not.toContain('Client');
    key(fixture, 'ArrowRight');
    await settle(fixture);
    expect(labels(fixture)).toContain('Client');
    key(fixture, 'ArrowRight'); // already expanded: move to the first child
    await settle(fixture);
    expect(focus().textContent).toContain('Software');
    key(fixture, 'ArrowLeft'); // collapse Software
    key(fixture, 'ArrowLeft'); // collapsed: move to the parent hive
    await settle(fixture);
    expect(focus().textContent).toContain('HKEY_CURRENT_USER');
    key(fixture, 'End');
    await settle(fixture);
    expect(focus().textContent).not.toContain('HKEY_CURRENT_USER');
    key(fixture, 'ArrowDown');
    key(fixture, 'Home');
    await settle(fixture);
    expect(focus().textContent).toContain('HKEY_CURRENT_USER');
    key(fixture, 'ArrowUp');
    key(fixture, 'Enter');
    key(fixture, ' ');
    key(fixture, 'x');
    await settle(fixture);
  });

  it('filters the tree', async () => {
    const { fixture } = await render();
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), 'ServerUrl');
    await settle(fixture);
    expect(labels(fixture)).toEqual(['HKEY_CURRENT_USER', 'Software', 'Contoso', 'Client', 'ServerUrl']);
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), 'nothing-here');
    await settle(fixture);
    expect(fixture.nativeElement.textContent).toContain('Nothing matches');
  });

  it('copies the path of the selected node', async () => {
    const { fixture, store } = await render();
    const copy = [...fixture.nativeElement.querySelectorAll('button')].find((b: HTMLButtonElement) => b.textContent?.includes('Copy path')) as HTMLButtonElement;
    expect(copy.disabled).toBe(true);
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    all(fixture, '[role=treeitem]').find((i) => i.textContent?.includes('ServerUrl'))!.click();
    await settle(fixture);
    expect(copy.disabled).toBe(false);
    copy.click();
    await settle(fixture);
    expect(writeText).toHaveBeenCalledWith('HKEY_CURRENT_USER\\Software\\Contoso\\Client\\ServerUrl');
    expect(copy.textContent).toContain('Copied');
    writeText.mockRejectedValue(new Error('denied'));
    await new Promise((r) => setTimeout(r, 1600));
    copy.click();
    await settle(fixture);
    expect(store.error()).toMatch(/Clipboard/);
  });

  it('opens large trees only down to the hives', async () => {
    const big = (): MockDataset => {
      const d = tinyDataset();
      d.sources.push({ id: 5, path: '/x/big.reg', name: 'big.reg', importer: 'registry', skipped: 0 });
      d.records.push(
        ...Array.from({ length: 450 }, (_, i) => ({ ...at(registryRecords(5), 0), fields: { Hive: 'HKLM', Key: `A\\K${i}`, ValueType: 'KEY' }, line: i + 1 })),
      );
      return d;
    };
    const { fixture } = await render(new MockBackend({ latency: false, dataset: big }));
    expect(labels(fixture)).toEqual(['HKLM', 'A']);
  });

  it('does not move to a parent from a top-level node and reports truncation', async () => {
    const backend = new MockBackend({ latency: false, dataset });
    const original = backend.queryPage.bind(backend);
    vi.spyOn(backend, 'queryPage').mockImplementation(async (req) => ({ ...(await original(req)), total: REGISTRY_ROW_CAP + 5 }));
    const { fixture } = await render(backend);
    expect(fixture.nativeElement.textContent).toContain('Showing the first');
    key(fixture, 'ArrowLeft');
    key(fixture, 'ArrowLeft');
    await settle(fixture);
    expect(el(fixture, '[role=treeitem][tabindex="0"]').textContent).toContain('HKEY_CURRENT_USER');
  });

  it('says so when there is no registry data and reports read errors', async () => {
    const { fixture } = await render(new MockBackend({ latency: false, dataset: tinyDataset }));
    expect(fixture.nativeElement.textContent).toContain('No registry data');
    const failing = new MockBackend({ latency: false, dataset });
    vi.spyOn(failing, 'queryPage').mockRejectedValue(new Error('disk'));
    TestBed.resetTestingModule();
    const r = await render(failing);
    expect(r.store.error()).toContain('Registry could not be read: disk');
  });
});
