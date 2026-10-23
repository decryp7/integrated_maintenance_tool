import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyDataset } from '../../testing/helpers';
import { at } from '../../testing/must';
import { RecordView, SourceInfo } from '../core/api.types';
import { DebriefBackend } from '../core/backend';
import { FilesBackend } from '../core/files-backend';
import { MockBackend } from '../core/mock/mock-backend';
import { EMPTY_QUERY, WorkspaceStore } from '../core/workspace.store';
import { RecordActions } from '../context/record-actions';
import { Inspector } from '../inspector/inspector';
import { Sidebar } from '../shell/sidebar';
import { ContextMenuHost } from '../ui/context-menu-host';
import { ContextMenuService } from '../ui/context-menu.service';
import { SourceFileActions } from './source-file-actions';

const source = (patch: Partial<SourceInfo>): SourceInfo => ({
  id: 0,
  path: '/logs/a.log',
  name: 'a.log',
  group: 'a.log',
  importer: 'text',
  records: 1,
  skipped: 0,
  dropped: 0,
  error: null,
  from: null,
  to: null,
  fileKind: 'text',
  ...patch,
});

/** The tiny dataset with its second file inside a zip archive. */
function archiveBackend(): MockBackend {
  const data = tinyDataset();
  data.sources = data.sources.map((s) => (s.id === 1 ? { ...s, path: '/logs/bundle.zip!/web02/app.log' } : s));
  return new MockBackend({ latency: false, dataset: () => data, pluginMarkerKinds: [] });
}

async function setup(backend: DebriefBackend = archiveBackend()) {
  TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
  const store = TestBed.inject(WorkspaceStore);
  await store.refreshSummary();
  const page = await TestBed.inject(DebriefBackend).queryPage({ query: EMPTY_QUERY, offset: 0, limit: 50, descending: false });
  return { store, files: TestBed.inject(SourceFileActions), rows: page.rows, backend };
}

const rowOf = (rows: RecordView[], source: number) => rows.find((r) => r.source === source)!;

describe('SourceFileActions', () => {
  it('offers open, open at line and show in folder for a text file', async () => {
    const { files } = await setup();
    expect(files.states(source({}), 12)).toEqual([
      { action: 'open', label: 'Open file', reason: null },
      { action: 'openAtLine', label: 'Open at line 12', reason: null },
      { action: 'reveal', label: 'Show in folder', reason: null },
    ]);
    expect(files.states(source({})).map((s) => s.action)).toEqual(['open', 'reveal']);
  });

  it('explains what is unavailable for archive members, compressed and binary files and gone sources', async () => {
    const { files } = await setup();
    const member = files.states(source({ fileKind: 'archiveMember' }), 3);
    expect(member.map((s) => [s.action, !!s.reason])).toEqual([
      ['open', true],
      ['openAtLine', true],
      ['reveal', false],
    ]);
    expect(member[0]?.reason).toMatch(/archive/);
    expect(member[2]?.label).toBe('Show archive in folder');
    expect(files.states(source({ fileKind: 'compressed' }), 3)[1]?.reason).toMatch(/Compressed/);
    expect(files.states(source({ fileKind: 'binary' }), 3)[1]?.reason).toMatch(/Not a text file/);
    expect(files.states(source({}), 0)[1]?.reason).toMatch(/no line number/);
    expect(files.states(null, 3).every((s) => s.reason?.includes('no longer open'))).toBe(true);
    const items = files.items(null, 3);
    expect(items.every((i) => i.disabled && i.hint)).toBe(true);
    items[0]?.run(); // a disabled entry does nothing even when run
  });

  it('asks the engine by source id and line, and reports refusals and fallbacks', async () => {
    const { files, store, rows } = await setup();
    const backend = TestBed.inject(FilesBackend);
    const spy = vi.spyOn(backend, 'openSourceFile');
    const text0 = rowOf(rows, 0);
    const s0 = files.sourceOf(text0)!;
    expect(await files.run(s0, 'openAtLine', text0.line)).toBe(true);
    expect(spy).toHaveBeenLastCalledWith({ sourceId: 0, action: 'openAtLine', line: text0.line });
    expect(await files.run(s0, 'reveal', 7)).toBe(true);
    expect(spy).toHaveBeenLastCalledWith({ sourceId: 0, action: 'reveal' });
    expect(store.error()).toBeNull();

    const member = files.sourceOf(rowOf(rows, 1))!;
    expect(await files.run(member, 'open')).toBe(false);
    expect(store.error()).toMatch(/inside the archive bundle\.zip/);

    spy.mockResolvedValueOnce({ with: 'defaultApp', path: '/logs/web01/app.clef' });
    expect(await files.run(s0, 'openAtLine', 4)).toBe(true);
    expect(store.notice()).toMatch(/VS Code was not found.*line 4/);
  });

  it('adds the file actions to the record menu, disabled with a reason inside an archive', async () => {
    const { rows } = await setup();
    const actions = TestBed.inject(RecordActions);
    const plain = actions.itemsFor(rowOf(rows, 0)).filter((i) => /Open file|Open at line|Show in folder/.test(i.label));
    expect(plain.map((i) => [i.label, !!i.disabled])).toEqual([
      ['Open file', false],
      [`Open at line ${rowOf(rows, 0).line}`, false],
      ['Show in folder', false],
    ]);
    const member = actions.itemsFor(rowOf(rows, 1));
    expect(member.find((i) => i.label === 'Open file')).toMatchObject({ disabled: true, hint: expect.stringMatching(/archive/) });
    expect(member.find((i) => i.label === 'Show archive in folder')?.disabled).toBe(false);
  });

  it('shows the reason under a disabled menu item', async () => {
    await setup();
    const fixture = TestBed.createComponent(ContextMenuHost);
    TestBed.inject(ContextMenuService).open(10, 10, TestBed.inject(SourceFileActions).items(source({ fileKind: 'archiveMember' }), 2));
    await settle(fixture);
    const items = all<HTMLButtonElement>(fixture, '[role=menuitem]');
    expect(items[0]?.disabled).toBe(true);
    expect(items[0]?.querySelector('.hint')?.textContent).toMatch(/extract it/);
    expect(items[2]?.disabled).toBe(false);
  });
});

describe('file actions in the inspector and the Explorer', () => {
  it('the inspector opens the event file at its line and explains a disabled action', async () => {
    const { store, rows } = await setup();
    const fixture = TestBed.createComponent(Inspector);
    const spy = vi.spyOn(TestBed.inject(FilesBackend), 'openSourceFile');
    store.select(rowOf(rows, 0));
    await settle(fixture);
    const line = rowOf(rows, 0).line;
    buttonByText(fixture, `Open at line ${line}`).click();
    expect(spy).toHaveBeenCalledWith({ sourceId: 0, action: 'openAtLine', line });
    expect(all(fixture, '[data-testid=file-reason]')).toHaveLength(0);

    store.select(rowOf(rows, 1));
    await settle(fixture);
    expect(buttonByText(fixture, 'Open file').disabled).toBe(true);
    expect(buttonByText(fixture, 'Show archive in folder').disabled).toBe(false);
    expect(el(fixture, '[data-testid=file-reason]').textContent).toMatch(/archive/);
  });

  it('the Explorer row menu (mouse or its keyboard button) offers the file actions', async () => {
    await setup();
    const fixture = TestBed.createComponent(Sidebar);
    const host = TestBed.createComponent(ContextMenuHost);
    await settle(fixture);
    const menu = TestBed.inject(ContextMenuService);
    at(all(fixture, '[data-testid=source-actions]'), 0).click();
    await settle(host);
    expect(menu.menu()?.items.map((i) => i.label)).toEqual(['Open file', 'Show in folder', 'Parse with…', 'Add note…', 'Adjust clock…', 'Remove']);
    menu.close();
    at(all(fixture, '.src'), 1).dispatchEvent(new MouseEvent('contextmenu', { clientX: 20, clientY: 30, bubbles: true }));
    expect(menu.menu()).toMatchObject({ x: 20, y: 30 });
    expect(menu.menu()?.items[1]?.label).toBe('Show archive in folder');
    await settle(host);
    expect(text(host)).toContain('Inside an archive');
  });
});
