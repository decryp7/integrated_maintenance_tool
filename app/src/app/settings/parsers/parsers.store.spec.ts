import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, tinyBackend } from '../../../testing/helpers';
import { ParserImportCheckDto } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { DialogService } from '../../ui/dialog.service';
import { MockParsersBackend } from './mock-parsers-backend';
import { ParserDialogs } from './parser-dialogs';
import { DEMO_PARSER_IMPORT, ParserFiles } from './parser-files';
import { ParsersBackend } from './parsers-backend';
import { BLANK_PARSER, initialChoice, ParsersStore } from './parsers.store';
import type { ParserLibraryDto } from '../../core/api.types';

function setup() {
  const imt = tinyBackend();
  const backend = new MockParsersBackend(imt);
  const dialogs = { confirmRemove: vi.fn().mockResolvedValue(true), confirmImport: vi.fn().mockResolvedValue(true), edit: vi.fn().mockResolvedValue(undefined) };
  const files = { pickImport: vi.fn().mockResolvedValue(DEMO_PARSER_IMPORT), pickExport: vi.fn().mockResolvedValue('/demo/exports/x.toml') };
  TestBed.configureTestingModule({
    providers: [provideTinyBackend(imt), { provide: ParsersBackend, useValue: backend }, { provide: ParserDialogs, useValue: dialogs }, { provide: ParserFiles, useValue: files }],
  });
  return { store: TestBed.inject(ParsersStore), backend, dialogs, files, imt };
}

describe('ParsersStore', () => {
  it('loads the library, grouped by category, ', async () => {
    const { store } = setup();
    expect(store.parsers()).toEqual([]);
    expect(store.stale()).toBe(0);
    await store.load();
    expect(store.loading()).toBe(false);
    expect(store.groups().map((g) => g.category)).toEqual(['.NET', 'Big data', 'Generic', 'Mobile', 'System', 'Web servers']);
    expect(store.broken()).toHaveLength(1);
    await store.setEnabled('nlog', false);
    expect(store.parsers().find((p) => p.id === 'nlog')?.enabled).toBe(false);
  });

  it('shows a failed listing and recovers on retry', async () => {
    const { store, backend } = setup();
    vi.spyOn(backend, 'list').mockRejectedValueOnce(new Error('engine gone'));
    await store.load();
    expect(store.loadError()).toBe('engine gone');
    await store.load();
    expect(store.loadError()).toBeNull();
    expect(store.library()).not.toBeNull();
  });

  it('removes user parsers and overrides after confirmation, never built-ins', async () => {
    const { store, backend, dialogs } = setup();
    await backend.save('schema = 1\nid = "mine"\nname = "Mine"\nversion = "1.0.0"\n');
    await backend.save((await backend.get('nlog')).toml, { previousId: 'nlog' });
    await store.load();
    await store.remove('csv');
    expect(dialogs.confirmRemove).not.toHaveBeenCalled();
    dialogs.confirmRemove.mockResolvedValueOnce(false);
    await store.remove('mine');
    expect(store.parsers().some((p) => p.id === 'mine')).toBe(true);
    await store.remove('mine');
    expect(store.parsers().some((p) => p.id === 'mine')).toBe(false);
    expect(store.notice()).toBe('Removed the parser Mine.');
    await store.remove('nlog');
    expect(store.parsers().find((p) => p.id === 'nlog')?.origin).toBe('builtin');
    expect(store.notice()).toContain('the built-in parser is used again');
    await store.remove('unknown');
    expect(store.busy()).toBeNull();
  });

  it('duplicates, edits (saving what the editor saved) and creates blank parsers', async () => {
    const { store, backend, dialogs } = setup();
    await store.load();
    await store.duplicate('nlog');
    expect(store.notice()).toBe('Saved a copy as nlog-copy.');
    expect(store.parsers().some((p) => p.id === 'nlog-copy')).toBe(true);

    await store.edit('serilog-text');
    expect(dialogs.edit).toHaveBeenCalledWith(expect.objectContaining({ id: 'serilog-text', origin: 'builtin' }), { readSources: [1] });
    expect(store.notice()).toBeNull();
    const saved = await backend.save('schema = 1\nid = "x"\nname = "X"\nversion = "1.0.0"\n');
    dialogs.edit.mockResolvedValueOnce(saved);
    await store.edit('nlog');
    expect(store.notice()).toBe('Saved the parser nlog.');
    expect(store.parsers().some((p) => p.id === 'x')).toBe(true);

    await store.newBlank();
    expect(dialogs.edit).toHaveBeenLastCalledWith({ id: '', origin: 'user', toml: BLANK_PARSER }, { isNew: true });
    dialogs.edit.mockResolvedValueOnce(saved);
    await store.newBlank();
    expect(store.notice()).toBe('Saved the new parser.');
    expect((await backend.validate(BLANK_PARSER)).ok).toBe(true);

    await store.edit('nope');
    expect(store.actionError()).toContain('no parser "nope"');
    store.dismiss();
    expect(store.actionError()).toBeNull();
  });

  it('saves for the format wizard and rejects like the engine', async () => {
    const { store } = setup();
    const lib = await store.save('schema = 1\nid = "w"\nname = "W"\nversion = "1.0.0"\n');
    expect(store.library()).toBe(lib);
    await expect(store.save('schema = 1\nid = "w"\nname = "W"\nversion = "1.0.1"\n')).rejects.toMatchObject({ code: 'fileExists' });
  });

  it('imports a new parser at once, and replaces one in the library only after the confirmation', async () => {
    const { store, dialogs, files, backend } = setup();
    await store.load();
    files.pickImport.mockResolvedValueOnce(null);
    await store.importFromFile();
    expect(store.notice()).toBeNull();
    await store.importFromFile();
    expect(store.notice()).toBe('Imported Inventory sync CSV 1.0.0.');
    expect(dialogs.confirmImport).not.toHaveBeenCalled();
    expect(store.stale()).toBe(0); // the tiny workspace has no inventory-*.csv

    const check = (relation: ParserImportCheckDto['relation']): ParserImportCheckDto => ({ toml: 'schema = 1\nid = "inventory-csv"\nname = "Inventory sync CSV"\nversion = "2.0.0"\n', id: 'inventory-csv', name: 'Inventory sync CSV', version: '2.0.0', relation, existing: null });
    const importCheck = vi.spyOn(backend, 'importCheck');
    const save = vi.spyOn(backend, 'save');
    importCheck.mockResolvedValueOnce(check('newer'));
    dialogs.confirmImport.mockResolvedValueOnce(false);
    await store.importFromFile();
    expect(save).not.toHaveBeenCalled();
    importCheck.mockResolvedValueOnce(check('newer'));
    await store.importFromFile();
    expect(save).toHaveBeenLastCalledWith(expect.stringContaining('2.0.0'), { previousId: 'inventory-csv', overwrite: true });
    expect(store.notice()).toBe('Updated Inventory sync CSV to 2.0.0.');
    importCheck.mockResolvedValueOnce(check('older'));
    await store.importFromFile();
    expect(store.notice()).toBe('Replaced Inventory sync CSV with version 2.0.0.');
    importCheck.mockRejectedValueOnce(new Error('missing field `id`'));
    await store.importFromFile();
    expect(store.actionError()).toBe('missing field `id`');
  });

  it('exports, asking before it replaces a file', async () => {
    const { store, files } = setup();
    await store.load();
    await store.exportParser('nlog');
    expect(store.notice()).toBe('Exported nlog to /demo/exports/x.toml.');
    const dialogs = TestBed.inject(DialogService);
    const again = store.exportParser('nlog');
    await vi.waitFor(() => expect(dialogs.stack().at(-1)?.options.title).toBe('Replace file?'));
    dialogs.stack().at(-1)!.ref.close();
    await again;
    expect(store.notice()).toBeNull();
    const replace = store.exportParser('nlog');
    await vi.waitFor(() => expect(dialogs.stack()).toHaveLength(1));
    dialogs.stack().at(-1)!.ref.close(true);
    await replace;
    expect(store.notice()).toContain('Exported nlog');
    files.pickExport.mockResolvedValueOnce(null);
    await store.exportParser('nlog');
    expect(store.notice()).toBeNull();
  });

  it('re-reads stale sources and "Parse with…" through import jobs the workspace follows', async () => {
    const { store, backend } = setup();
    const workspace = TestBed.inject(WorkspaceStore);
    await workspace.init();
    await backend.save('schema = 1\nid = "web-log"\nname = "Web log"\nversion = "1.0.0"\n[match]\nfiles = ["app.log"]\npriority = 5\n[format]\nkind = "text"\n');
    await store.load();
    expect(store.stale()).toBe(1);
    await store.reread();
    expect(store.stale()).toBe(0);
    expect(store.notice()).toBe('Read the files again with their parsers.');
    expect(workspace.summary()?.sources[1]?.parser).toBe('web-log');
    await store.assign(1, 'generic');
    expect(workspace.summary()?.sources[1]?.parser).toBe('generic');
    expect(store.parsers().find((p) => p.id === 'generic')?.sources.map((s) => s.id)).toEqual([1]);
  });

  it('Parse with… (I3): a source is chosen for, the library offers each parser, and a choice or Cancel ends it', async () => {
    const { store, backend } = setup();
    const workspace = TestBed.inject(WorkspaceStore);
    await workspace.init();
    const source = workspace.summary()!.sources[1]!;
    expect(store.targetChoice()).toBeNull();
    await store.readTargetWith('nlog');
    expect(store.readTarget()).toBeNull();

    await store.chooseFor(source);
    expect(store.readTarget()).toEqual({ id: source.id, name: source.name });
    // Matched automatically: Automatic is the starting choice.
    expect(store.targetChoice()).toBe('');
    store.cancelChoice();
    expect(store.readTarget()).toBeNull();
    expect(store.targetChoice()).toBeNull();

    const assign = vi.spyOn(backend, 'assign');
    await store.chooseFor(source);
    await store.readTargetWith('nlog');
    expect(assign).toHaveBeenLastCalledWith(source.id, 'nlog');
    expect(store.readTarget()).toBeNull();
    expect(workspace.summary()?.sources[1]?.parser).toBe('nlog');

    // A parser the user chose before is the starting choice again.
    await store.chooseFor(source);
    expect(store.targetChoice()).toBe('nlog');
    await store.readTargetWith(null);
    expect(assign).toHaveBeenLastCalledWith(source.id, null);
  });

  it('starts on what the user chose before, else Automatic', () => {
    const lib = (chosen: boolean) =>
      ({ parsers: [{ id: 'nlog', sources: [{ id: 3, name: 'a.log', chosen }] }], broken: [], stale: 0 }) as unknown as ParserLibraryDto;
    expect(initialChoice(null, 3)).toBe('');
    expect(initialChoice(lib(false), 3)).toBe('');
    expect(initialChoice(lib(true), 3)).toBe('nlog');
    expect(initialChoice(lib(true), 4)).toBe('');
  });
});
