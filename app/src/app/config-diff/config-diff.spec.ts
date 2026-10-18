import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, MemoryLogger, settle, text, typeInto } from '../../testing/helpers';
import { Logger } from '../core/logger';
import { CHANGE_KINDS, DiffOptions, SnapshotFile } from '../core/config-diff.types';
import { IMT_COMMANDS, IMT_VIEWS, ImtCommand, ImtView } from '../extensions/tokens';
import { ConfigDiffBackend, provideConfigDiffBackend, TauriConfigDiffBackend } from './config-diff-backend';
import { ConfigDiffPicker, Side } from './config-diff-picker';
import { ConfigDiffView, valueText } from './config-diff-view';
import { ConfigDiffStore, PAGE_SIZE, parsePatterns } from './config-diff.store';
import { demoEntries, MOCK_DEFAULT_IGNORE, MockConfigDiffBackend, mockPage } from './mock/mock-config-diff-backend';
import { CONFIG_DIFF_COMMANDS, CONFIG_DIFF_VIEWS, provideConfigDiff } from './provide-config-diff';
import { at } from '../../testing/must';

class FakePicker {
  file: Record<Side, string | null> = { left: '/l/a.json', right: '/r/a.json' };
  folder: Record<Side, string | null> = { left: '/snap/mon', right: '/snap/tue' };
  exportPath: string | null = '/out/diff.csv';
  pickFile = async (side: Side) => this.file[side];
  pickFolder = async (side: Side) => this.folder[side];
  pickExport = async () => this.exportPath;
}

/** Rows of the demo diff with the default ignore patterns, and with none. */
const WITH_DEFAULTS = 219;
const WITHOUT_IGNORE = 220;

function options(patch: Partial<DiffOptions['view']> = {}, ignore: string[] = MOCK_DEFAULT_IGNORE): DiffOptions {
  return { compare: { ignore, caseInsensitive: false }, view: { kinds: [...CHANGE_KINDS], search: '', expand: 'all', toggled: [], offset: 0, limit: 500, ...patch } };
}

function setup() {
  const backend = new MockConfigDiffBackend();
  const picker = new FakePicker();
  TestBed.configureTestingModule({
    providers: [
      { provide: ConfigDiffBackend, useValue: backend },
      { provide: ConfigDiffPicker, useValue: picker },
      { provide: Logger, useFactory: () => new MemoryLogger() },
    ],
  });
  return { backend, picker, store: TestBed.inject(ConfigDiffStore) };
}

describe('TauriConfigDiffBackend', () => {
  it('maps each operation to its command and arguments', async () => {
    const calls: [string, unknown][] = [];
    const call = (async (cmd: string, args?: unknown) => {
      calls.push([cmd, args]);
      return null;
    }) as never;
    const b = new TauriConfigDiffBackend(call);
    const o = options();
    await b.formats();
    await b.diff('/a', '/b', o);
    await b.snapshots('/x', '/y', o.compare);
    await b.exportDiff('/a', '/b', o, 'html', '/o.html');
    expect(calls).toEqual([
      ['config_diff_formats', undefined],
      ['diff_configs', { leftPath: '/a', rightPath: '/b', options: o }],
      ['diff_snapshots', { leftDir: '/x', rightDir: '/y', compare: o.compare }],
      ['export_config_diff', { leftPath: '/a', rightPath: '/b', options: o, format: 'html', path: '/o.html' }],
    ]);
  });
});

describe('mock config diff', () => {
  it('ignores, filters, searches, folds and pages like the engine', () => {
    const all = mockPage(demoEntries(), options());
    expect(all.counts).toEqual({ added: 4, removed: 1, changed: 7, unchanged: 207, ignored: 1 });
    expect(all.total).toBe(WITH_DEFAULTS);
    expect(mockPage(demoEntries(), options({}, [])).total).toBe(WITHOUT_IGNORE);
    expect(all.rows.find((r) => r.path === 'Servers[0]/port')).toMatchObject({ depth: 2, name: 'port', kind: 'changed' });
    expect(all.rows.find((r) => r.path === 'Servers[0]')).toMatchObject({ depth: 1, name: '[0]', expandable: true, expanded: true, hasChanges: true });
    const changed = mockPage(demoEntries(), options({ kinds: ['changed'] }));
    expect(changed.rows.filter((r) => !r.context).every((r) => r.kind === 'changed')).toBe(true);
    expect(changed.rows.find((r) => r.path === 'Logging')?.context).toBe(true);
    const search = mockPage(demoEntries(), options({ search: 'WEB02' }));
    expect(search.rows.map((r) => r.path)).toEqual(['Servers', 'Servers[1]', 'Servers[1]/host']);
    const folded = mockPage(demoEntries(), options({ expand: 'changedOnly' }));
    expect(folded.rows.find((r) => r.path === 'Logging/Sources')?.expanded).toBe(true);
    const toggled = mockPage(demoEntries(), options({ toggled: ['Logging'] }));
    expect(toggled.rows.some((r) => r.path.startsWith('Logging/'))).toBe(false);
    const regex = mockPage(demoEntries(), options({}, ['re:^Logging$', '', '?eatures']));
    expect(regex.rows.some((r) => r.path.startsWith('Logging') || r.path.startsWith('Features'))).toBe(false);
    const page = mockPage(demoEntries(), options({ offset: 215, limit: 10 }));
    expect(page.offset).toBe(215);
    expect(page.rows.length).toBe(all.total - 215);
  });

  it('fails like unreadable files and answers the other operations', async () => {
    const b = new MockConfigDiffBackend();
    await expect(b.diff('/missing.json', '/b', options())).rejects.toContain('No such file');
    await expect(b.diff('/a', '/broken.json', options())).rejects.toContain('right file: JSON');
    await expect(b.diff('/a', '/b', options({}, ['re:(']))).rejects.toContain('regex');
    expect((await b.formats()).formats.map((f) => f.id)).toContain('registry');
    const snap = await b.snapshots('/a', '/b', { ignore: [], caseInsensitive: false });
    expect(snap.files.map((f) => f.status)).toEqual(['changed', 'changed', 'unchanged', 'removed', 'added', 'unreadable']);
    await expect(b.snapshots('/missing', '/b', { ignore: [], caseInsensitive: false })).rejects.toContain('left folder');
    expect((await b.exportDiff('/a', '/b', options({ kinds: ['added'] }), 'csv', '/o.csv')).rows).toBe(4);
    expect(b.calls).toContain('export:csv:/o.csv');
  });
});

describe('ConfigDiffStore', () => {
  it('loads defaults, compares two chosen files and pages through the rows', async () => {
    const { store, backend } = setup();
    await store.init();
    expect(store.ignoreText()).toBe(MOCK_DEFAULT_IGNORE.join('\n'));
    expect(store.formats().length).toBe(7);
    await store.choose('left');
    expect(store.meta()).toBeNull();
    await store.choose('right');
    expect(store.ready()).toBe(true);
    expect(store.total()).toBe(WITH_DEFAULTS);
    expect(store.rowAt(0)?.path).toBe('ConnectionStrings');
    expect(store.rowAt(PAGE_SIZE)).toBeNull();
    await store.ensure(PAGE_SIZE - 5, PAGE_SIZE + 5);
    expect(store.rowAt(PAGE_SIZE)).not.toBeNull();
    expect(backend.calls.filter((c) => c.startsWith('diff:'))).toEqual(['diff:0', `diff:${PAGE_SIZE}`]);
    // Already loaded pages are not fetched again.
    await store.ensure(0, 10);
    await store.ensure(5, 5);
    expect(backend.calls.filter((c) => c.startsWith('diff:')).length).toBe(2);
    store.select(2);
    expect(store.selectedRow()?.path).toBe('ConnectionStrings/Orders');
    store.select(null);
    expect(store.selectedRow()).toBeNull();
  });

  it('refetches when filters, search, folding, ignore patterns or case change', async () => {
    const { store } = setup();
    await store.init();
    store.setPath('left', '/a.json');
    store.setPath('right', '/b.json');
    await store.compare();
    await store.toggleKind('unchanged');
    expect(store.kinds()).toEqual(['added', 'removed', 'changed']);
    expect(store.total()).toBeLessThan(40);
    await store.toggleKind('unchanged');
    expect(store.kinds()).toEqual([...CHANGE_KINDS]);
    await store.setSearch('web02');
    expect(store.total()).toBe(3);
    await store.setSearch('web02');
    await store.setSearch('');
    await store.setExpand('changedOnly');
    expect(store.expand()).toBe('changedOnly');
    await store.toggle('Logging');
    expect(store.toggled()).toEqual(['Logging']);
    await store.toggle('Logging');
    expect(store.toggled()).toEqual([]);
    await store.setIgnoreText('*Install*\n\n Logging \n');
    expect(store.ignore()).toEqual(['*Install*', 'Logging']);
    expect(store.meta()?.counts.ignored).toBeGreaterThan(200);
    await store.resetIgnore();
    expect(store.ignore()).toBeNull();
    await store.setCaseInsensitive(true);
    expect(store.meta()?.caseInsensitive).toBe(true);
    await store.swap();
    expect(store.left()).toBe('/b.json');
    store.setLayout('unified');
    expect(store.layout()).toBe('unified');
  });

  it('shows errors, drops stale answers and retries', async () => {
    const { store, backend } = setup();
    store.setPath('left', '/missing.json');
    store.setPath('right', '/b.json');
    await store.compare();
    expect(store.error()).toContain('No such file');
    expect(store.meta()).toBeNull();
    store.dismiss();
    expect(store.error()).toBeNull();
    // A slow answer for an older view is ignored.
    store.setPath('left', '/a.json');
    let release: () => void = () => undefined;
    const slow = vi.spyOn(backend, 'diff').mockImplementationOnce(async (...args) => {
      await new Promise<void>((r) => (release = r));
      return MockConfigDiffBackend.prototype.diff.apply(backend, args);
    });
    const first = store.compare();
    const second = store.compare();
    release();
    await Promise.all([first, second]);
    expect(slow).toHaveBeenCalledTimes(2);
    expect(store.total()).toBe(WITHOUT_IGNORE);
    // A failed format list is reported too.
    vi.spyOn(backend, 'formats').mockRejectedValueOnce('no engine');
    await store.init();
    expect(store.error()).toBe('no engine');
    // Without both files nothing is fetched.
    store.setPath('right', ' ');
    await store.compare();
    await store.ensure(0, 10);
  });

  it('copies, exports and reports problems', async () => {
    const { store, picker, backend } = setup();
    store.setPath('left', '/a.json');
    store.setPath('right', '/b.json');
    await store.compare();
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    await store.copy('path', 'Servers[0]/port');
    expect(writeText).toHaveBeenCalledWith('Servers[0]/port');
    expect(store.notice()).toBe('Copied path');
    writeText.mockRejectedValueOnce(new Error('denied'));
    await store.copy('path', 'x');
    expect(store.notice()).toBe('The clipboard is not available');
    await store.exportAs('csv');
    expect(store.notice()).toBe(`Exported ${WITHOUT_IGNORE} rows to /out/diff.csv`);
    picker.exportPath = null;
    await store.exportAs('html');
    expect(backend.calls.filter((c) => c.startsWith('export')).length).toBe(1);
    picker.exportPath = '/out/x.html';
    vi.spyOn(backend, 'exportDiff').mockResolvedValueOnce({ path: '/out/x.html', rows: 1 });
    await store.exportAs('html');
    expect(store.notice()).toBe('Exported 1 row to /out/x.html');
    vi.spyOn(backend, 'exportDiff').mockRejectedValueOnce('disk full');
    await store.exportAs('html');
    expect(store.error()).toBe('Could not export the diff: disk full');
  });

  it('compares snapshot folders and opens a file pair', async () => {
    const { store, picker } = setup();
    picker.file.left = null;
    await store.choose('left');
    expect(store.left()).toBe('');
    await store.chooseFolder('left');
    expect(store.snapshot()).toBeNull();
    picker.folder.right = null;
    await store.chooseFolder('right');
    picker.folder.right = '/snap/tue';
    await store.chooseFolder('right');
    expect(store.snapshot()?.files.length).toBe(6);
    const changed = at(store.snapshot()!.files, 0);
    await store.openFile(changed);
    expect(store.mode()).toBe('files');
    expect(store.left()).toBe('/snap/mon/app/appsettings.json');
    expect(store.total()).toBe(WITHOUT_IGNORE);
    await store.openFile({ ...changed, leftPath: null } as SnapshotFile);
    store.setDir('left', '/missing');
    await store.compareFolders();
    expect(store.snapshotError()).toContain('left folder');
    expect(store.snapshot()).toBeNull();
    store.setMode('folders');
    expect(store.mode()).toBe('folders');
  });

  it('parses ignore pattern text', () => {
    expect(parsePatterns(' a \n\n b\n')).toEqual(['a', 'b']);
  });
});

describe('ConfigDiffView', () => {
  async function view() {
    const ctx = setup();
    const fixture = TestBed.createComponent(ConfigDiffView);
    await settle(fixture);
    return { ...ctx, fixture };
  }

  it('compares chosen files and renders a virtual window of the tree', async () => {
    const { fixture, store } = await view();
    expect(text(fixture)).toContain('Choose two configuration files to compare');
    at(all<HTMLButtonElement>(fixture, 'button').filter((b) => b.textContent?.trim() === 'Choose…'), 0).click();
    await settle(fixture);
    at(all<HTMLButtonElement>(fixture, 'button').filter((b) => b.textContent?.trim() === 'Choose…'), 1).click();
    await settle(fixture);
    expect(store.total()).toBe(WITH_DEFAULTS);
    const rows = all(fixture, '[role=row][data-kind]');
    expect(rows.length).toBeGreaterThan(10);
    expect(rows.length).toBeLessThan(60);
    expect(el(fixture, '[role=treegrid]').getAttribute('aria-rowcount')).toBe(String(WITH_DEFAULTS));
    expect(text(fixture)).toContain('Server=db02;Database=Orders');
    expect(el(fixture, '.kind[data-kind=added] .count').textContent).toBe('4');
    // Scrolling far loads the next page.
    const tree = el<HTMLElement>(fixture, '.tree');
    tree.scrollTop = 210 * 24;
    tree.dispatchEvent(new Event('scroll'));
    await settle(fixture);
    expect(all(fixture, '[role=row][data-kind]').some((r) => r.textContent?.includes('Timeout'))).toBe(true);
  });

  it('filters, searches, folds, switches layout and copies the selected row', async () => {
    const { fixture, store } = await view();
    store.setPath('left', '/a.json');
    store.setPath('right', '/b.json');
    await settle(fixture);
    buttonByText(fixture, 'Compare').click();
    await settle(fixture);
    el<HTMLInputElement>(fixture, '.kind[data-kind=unchanged] input').dispatchEvent(new Event('change'));
    await settle(fixture);
    expect(store.kinds()).not.toContain('unchanged');
    vi.useFakeTimers();
    typeInto(el<HTMLInputElement>(fixture, 'input[type=search]'), 'web02');
    vi.advanceTimersByTime(300);
    vi.useRealTimers();
    await settle(fixture);
    expect(store.search()).toBe('web02');
    const box = all<HTMLInputElement>(fixture, '.bar .check input').find((i) => !i.closest('.kind'))!;
    box.checked = true;
    box.dispatchEvent(new Event('change'));
    await settle(fixture);
    expect(store.expand()).toBe('changedOnly');
    el<HTMLButtonElement>(fixture, 'button.tog').click();
    await settle(fixture);
    expect(store.toggled().length).toBe(1);
    buttonByText(fixture, 'Unified').click();
    await settle(fixture);
    expect(el(fixture, '.tree').classList).toContain('unified');
    expect(all(fixture, '.one').length).toBeGreaterThan(0);
    // Select a row and copy.
    at(all<HTMLElement>(fixture, '[role=row][data-kind]'), 0).click();
    await settle(fixture);
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    buttonByText(fixture, 'Copy path').click();
    await settle(fixture);
    expect(writeText).toHaveBeenCalled();
    expect(text(fixture)).toContain('Copied path');
  });

  it('moves the selection and expands with the keyboard', async () => {
    const { fixture, store } = await view();
    const tree = el<HTMLElement>(fixture, '.tree');
    tree.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }));
    store.setPath('left', '/a.json');
    store.setPath('right', '/b.json');
    await store.compare();
    await settle(fixture);
    const key = async (k: string) => {
      tree.dispatchEvent(new KeyboardEvent('keydown', { key: k, cancelable: true }));
      await settle(fixture);
    };
    await key('ArrowUp');
    expect(store.selected()).toBe(0);
    await key('ArrowDown');
    expect(store.selected()).toBe(1);
    await key('Home');
    expect(store.selected()).toBe(0);
    await key('ArrowLeft');
    expect(store.toggled()).toEqual(['ConnectionStrings']);
    await key('ArrowRight');
    expect(store.toggled()).toEqual([]);
    await key('Enter');
    expect(store.toggled()).toEqual(['ConnectionStrings']);
    await key('Enter');
    await key('PageDown');
    expect(store.selected()).toBeGreaterThan(5);
    await key('PageUp');
    expect(store.selected()).toBe(0);
    await key('End');
    expect(store.selected()).toBe(store.total() - 1);
    await key('ArrowDown');
    expect(store.selected()).toBe(store.total() - 1);
    await key('x');
    expect(el(fixture, '[role=treegrid]').getAttribute('aria-activedescendant')).toBe(`cd-row-${store.total() - 1}`);
    store.select(null);
    await key('ArrowDown');
    expect(store.selected()).toBe(0);
  });

  it('shows errors with Retry, the options and the folder comparison', async () => {
    const { fixture, store } = await view();
    store.setPath('left', '/missing.json');
    store.setPath('right', '/b.json');
    await store.compare();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('No such file');
    typeInto(el<HTMLInputElement>(fixture, '.pick input'), '/a.json', 'change');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(store.error()).toBeNull();
    typeInto(el<HTMLTextAreaElement>(fixture, 'textarea'), 'Logging', 'change');
    await settle(fixture);
    expect(store.ignore()).toEqual(['Logging']);
    buttonByText(fixture, 'Reset to defaults').click();
    const ci = el<HTMLInputElement>(fixture, '.row-opts input[type=checkbox]');
    ci.checked = true;
    ci.dispatchEvent(new Event('change'));
    await settle(fixture);
    expect(store.caseInsensitive()).toBe(true);
    expect(text(fixture)).toContain('Formats: JSON');
    buttonByText(fixture, '⇄').click();
    await settle(fixture);
    buttonByText(fixture, 'Export CSV').click();
    await settle(fixture);
    expect(text(fixture)).toContain('Exported');
    // Folders.
    buttonByText(fixture, 'Two snapshot folders').click();
    await settle(fixture);
    expect(text(fixture)).toContain('Choose two folders');
    typeInto(at(all<HTMLInputElement>(fixture, '.pick input'), 0), '/snap/a', 'change');
    typeInto(at(all<HTMLInputElement>(fixture, '.pick input'), 1), '/snap/b', 'change');
    await settle(fixture);
    buttonByText(fixture, 'Compare folders').click();
    await settle(fixture);
    expect(all(fixture, '.files tbody tr').length).toBe(6);
    expect(text(fixture)).toContain('EOF while parsing');
    at(all<HTMLButtonElement>(fixture, '.files button'), 0).click();
    await settle(fixture);
    expect(store.mode()).toBe('files');
    expect(store.left()).toBe('/snap/a/app/appsettings.json');
    buttonByText(fixture, 'Two snapshot folders').click();
    store.setDir('left', '/missing');
    await settle(fixture);
    buttonByText(fixture, 'Compare folders').click();
    await settle(fixture);
    expect(text(fixture)).toContain('left folder');
  });

  it('shows changed values inline when unified, and empty states', async () => {
    const { fixture, store, backend } = await view();
    store.setLayout('unified');
    store.setPath('left', '/a.json');
    store.setPath('right', '/b.json');
    await store.setSearch('Orders');
    await settle(fixture);
    expect(all(fixture, '.one .old').map((e) => e.textContent)).toContain('Server=db01;Database=Orders');
    await store.setSearch('no such text anywhere');
    await settle(fixture);
    expect(text(fixture)).toContain('No differences to show with these filters');
    store.setMode('folders');
    let release: () => void = () => undefined;
    vi.spyOn(backend, 'snapshots').mockImplementationOnce(async (l, r) => {
      await new Promise<void>((res) => (release = res));
      return { left: l, right: r, files: [], truncated: true };
    });
    store.setDir('left', '/a');
    store.setDir('right', '/b');
    const done = store.compareFolders();
    await settle(fixture);
    expect(text(fixture)).toContain('Comparing folders');
    release();
    await done;
    await settle(fixture);
    expect(text(fixture)).toContain('No configuration files found');
    expect(text(fixture)).toContain('Only the first files');
  });

  it('formats values', () => {
    expect(valueText(null)).toBe('');
    expect(valueText({ kind: 'section', text: '', clipped: false })).toBe('');
    expect(valueText({ kind: 'string', text: '', clipped: false })).toBe('""');
    expect(valueText({ kind: 'integer', text: '5', clipped: false })).toBe('5');
  });
});

describe('provideConfigDiff', () => {
  it('registers the view, the command and a browser backend', async () => {
    TestBed.configureTestingModule({ providers: [provideConfigDiff(), { provide: Logger, useFactory: () => new MemoryLogger() }] });
    const views = TestBed.inject(IMT_VIEWS).flat() as ImtView[];
    expect(views.map((v) => v.id)).toContain('config-diff');
    const commands = TestBed.inject(IMT_COMMANDS).flat() as ImtCommand[];
    const cmd = commands.find((c) => c.id === 'tools.configDiff.show')!;
    const showView = vi.fn();
    await cmd.run({ showView } as never);
    expect(showView).toHaveBeenCalledWith('config-diff');
    expect(await CONFIG_DIFF_VIEWS[0]?.component()).toBe(ConfigDiffView);
    expect(CONFIG_DIFF_COMMANDS.length).toBe(1);
    expect((await TestBed.inject(ConfigDiffBackend).formats()).formats.length).toBe(7);
    expect(provideConfigDiffBackend().length).toBe(1);
  });
});

describe('ConfigDiffPicker (browser)', () => {
  it('returns demo paths', async () => {
    const p = new ConfigDiffPicker();
    expect(await p.pickFile('left')).toBe('/demo/config/left/appsettings.json');
    expect(await p.pickFolder('right')).toBe('/demo/snapshots/tuesday');
    expect(await p.pickFolder('left')).toBe('/demo/snapshots/monday');
    expect(await p.pickExport('html')).toBe('/demo/exports/config-diff.html');
  });
});
