import { TestBed } from '@angular/core/testing';
import { provideTinyBackendBare, tinyBackend } from '../../../testing/helpers';
import { DebriefBackend } from '../../core/backend';
import { CmdError } from '../../core/cmd-error';
import { mockJobs } from '../../jobs/mock-jobs';
import { MockParsersBackend } from './mock-parsers-backend';
import { ParsersBackend, TauriParsersBackend } from './parsers-backend';
import { DEMO_PARSER_IMPORT } from './parser-files';

describe('ParsersBackend', () => {
  it('maps every method to its Tauri command and arguments', async () => {
    const calls: [string, unknown][] = [];
    const t = new TauriParsersBackend((async (cmd: string, args?: unknown) => (calls.push([cmd, args]), null)) as never);
    await t.list();
    await t.get('nlog');
    await t.validate('schema = 1');
    await t.save('schema = 1');
    await t.save('schema = 1', { previousId: 'a', overwrite: true });
    await t.duplicate('nlog');
    await t.remove('mine');
    await t.setEnabled('nlog', false);
    await t.importCheck('/p/x.toml');
    await t.exportTo('nlog', '/p/nlog.toml');
    await t.exportTo('nlog', '/p/nlog.toml', true);
    await t.draft({ fileName: 'a.log', base: null, regex: '^x', template: null, timestampFormat: null });
    await t.reread();
    await t.assign(3, 'nlog');
    await t.assign(3, null);
    expect(calls).toEqual([
      ['parsers_list', undefined],
      ['parser_get', { id: 'nlog' }],
      ['parser_validate', { toml: 'schema = 1' }],
      ['parser_save', { toml: 'schema = 1', previousId: null, overwrite: false }],
      ['parser_save', { toml: 'schema = 1', previousId: 'a', overwrite: true }],
      ['parser_duplicate', { id: 'nlog' }],
      ['parser_remove', { id: 'mine' }],
      ['parser_set_enabled', { id: 'nlog', enabled: false }],
      ['parser_import_check', { path: '/p/x.toml' }],
      ['parser_export', { id: 'nlog', path: '/p/nlog.toml', overwrite: false }],
      ['parser_export', { id: 'nlog', path: '/p/nlog.toml', overwrite: true }],
      ['parser_draft', { request: { fileName: 'a.log', base: null, regex: '^x', template: null, timestampFormat: null } }],
      ['parsers_reread', undefined],
      ['parser_assign', { sourceId: 3, parserId: 'nlog' }],
      ['parser_assign', { sourceId: 3, parserId: null }],
    ]);
  });

  it('is provided in root: the mock over the mock workspace in a browser', async () => {
    TestBed.configureTestingModule({ providers: provideTinyBackendBare() });
    const lib = await TestBed.inject(ParsersBackend).list();
    expect(lib.parsers.find((p) => p.id === 'serilog-clef')?.sources.map((s) => s.name)).toEqual(['app.clef']);
    expect(lib.parsers.find((p) => p.id === 'serilog-text')?.sources.map((s) => s.name)).toEqual(['app.log']);
  });
});

describe('MockParsersBackend', () => {
  it('follows the mock workspace: import, stale sources, re-read and the summary', async () => {
    const debrief = tinyBackend();
    const b = new MockParsersBackend(debrief);
    const check = await b.importCheck(DEMO_PARSER_IMPORT);
    expect(check).toMatchObject({ id: 'inventory-csv', relation: 'new', existing: null });
    await b.save(check.toml);
    // Taking the .log file from serilog-text: enable a parser that reads *.log better.
    await b.save('schema = 1\nid = "web-log"\nname = "Web log"\nversion = "1.0.0"\n[match]\nfiles = ["app.log"]\npriority = 5\n[format]\nkind = "text"\n');
    expect((await b.list()).stale).toBe(1);
    const job = await b.reread();
    await mockJobs().wait(job);
    const lib = await b.list();
    expect(lib.stale).toBe(0);
    expect(lib.parsers.find((p) => p.id === 'web-log')?.sources.map((s) => s.name)).toEqual(['app.log']);
    expect((await debrief.workspaceSummary()).sources.map((s) => s.parser)).toEqual(['serilog-clef', 'web-log']);
  });

  it('assigns a parser to one source, and back to the automatic choice', async () => {
    const debrief = tinyBackend();
    const b = new MockParsersBackend(debrief);
    await mockJobs().wait(await b.assign(1, 'generic'));
    expect((await debrief.workspaceSummary()).sources[1]?.parser).toBe('generic');
    await mockJobs().wait(await b.assign(1, null));
    expect((await debrief.workspaceSummary()).sources[1]?.parser).toBe('serilog-text');
    await expect(b.assign(9, null)).rejects.toThrow('no longer open');
    await expect(b.assign(1, 'nope')).rejects.toBeInstanceOf(CmdError);
  });

  it('refuses files that are not parser files and keeps its own library over another backend', async () => {
    const b = new MockParsersBackend({} as DebriefBackend);
    await expect(b.importCheck('/demo/x.json')).rejects.toThrow('not a parser file');
    const lib = await b.list();
    expect(lib.parsers.length).toBeGreaterThan(10);
    expect(lib.parsers.every((p) => p.sources.length === 0)).toBe(true);
    expect(lib.broken).toHaveLength(1);
    expect((await b.setEnabled('nlog', false)).parsers.find((p) => p.id === 'nlog')?.enabled).toBe(false);
    const copy = await b.duplicate('nlog');
    expect((await b.get(copy.id)).origin).toBe('user');
    expect((await b.remove(copy.id)).parsers.some((p) => p.id === copy.id)).toBe(false);
    expect((await b.validate('schema = 2')).error?.message).toContain('schema 2');
    await b.exportTo('nlog', '/demo/exports/nlog.toml');
    await expect(b.exportTo('nlog', '/demo/exports/nlog.toml')).rejects.toThrow('already exists');
    expect((await b.draft({ fileName: 'a.log', base: null, regex: '^x', template: null, timestampFormat: null })).id).toBe('a.log');
  });
});
