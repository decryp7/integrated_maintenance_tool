import { CmdError } from '../cmd-error';
import { compareVersions, mockGlob, mockHash, MockParserLibrary, MockParserSource, mockParseParser } from './mock-parsers';
import { MOCK_BUILTIN_PARSERS, MOCK_IMPORT_PARSER_PATH, MOCK_PARSER_DIR, MOCK_PARSER_FILES } from './mock-parsers-data';
import { mockPreviewParse } from './mock-parse';

const SOURCES: MockParserSource[] = [
  { id: 0, path: '/i/web01/orders.clef', name: 'orders.clef', importer: 'json' },
  { id: 1, path: '/i/worker/worker.log', name: 'worker.log', importer: 'text' },
  { id: 2, path: '/i/sync/inventory-sync.csv', name: 'inventory-sync.csv', importer: 'csv' },
  { id: 3, path: '/i/system/export.reg', name: 'export.reg', importer: 'registry' },
];

const parser = (id: string, extra = '') => `schema = 1\nid = "${id}"\nname = "P ${id}"\nversion = "1.0.0"\n${extra}`;

describe('mockParseParser', () => {
  it('reads the identity, match and format of a parser', () => {
    const r = mockParseParser(MOCK_BUILTIN_PARSERS.get('serilog-text')!);
    expect(r).toEqual({ ok: true, parser: expect.objectContaining({ id: 'serilog-text', kind: 'text', priority: 0, files: ['*.log', '*.txt'], category: '.NET' }) });
    expect(mockParseParser(parser('bare'))).toMatchObject({ ok: true, parser: { kind: 'auto', category: 'Custom', description: '', files: null } });
  });

  it('refuses what the engine refuses, with a line and column', () => {
    expect(mockParseParser('schema = 1\nid = [')).toMatchObject({ ok: false, error: { line: 2 } });
    expect(mockParseParser('id = "x"')).toMatchObject({ ok: false, error: { message: 'missing field `schema`', line: 1 } });
    expect(mockParseParser('\nschema = 2').ok ? null : mockParseParser('\nschema = 2')).toMatchObject({ error: { message: expect.stringContaining('update IMT'), line: 2 } });
    expect(mockParseParser('schema = 1\nname = "x"')).toMatchObject({ error: { message: 'missing field `id`' } });
    expect(mockParseParser('schema = 1\nid = "Bad Id"')).toMatchObject({ error: { message: expect.stringContaining('invalid id'), line: 2 } });
    expect(mockParseParser('schema = 1\nid = "x"\nversion = "1.0.0"')).toMatchObject({ error: { message: 'missing field `name`' } });
    expect(mockParseParser('schema = 1\nid = "x"\nname = "X"')).toMatchObject({ error: { message: 'missing field `version`' } });
    expect(mockParseParser('schema = 1\nid = "x"\nname = "X"\nversion = "one"')).toMatchObject({ error: { message: expect.stringContaining('semver'), line: 4 } });
  });
});

describe('mock parser helpers', () => {
  it('compares semver versions', () => {
    expect(compareVersions('1.1.0', '1.0.9')).toBeGreaterThan(0);
    expect(compareVersions('1.0.0', '1.0.0-beta')).toBe(0);
    expect(compareVersions('0.9.0', '1.0.0')).toBeLessThan(0);
    expect(compareVersions('x', '0.0.0')).toBe(0);
  });

  it('hashes text and matches globs on names and paths', () => {
    expect(mockHash('a')).toMatch(/^[0-9a-f]{8}$/);
    expect(mockHash('a')).not.toBe(mockHash('b'));
    const s = { path: '/logs/web/u_ex261014.log', name: 'u_ex261014.log' };
    expect(mockGlob('u_ex*.log', s)).toBe(true);
    expect(mockGlob('logs/**/*.log', s)).toBe(true);
    expect(mockGlob('*.csv', s)).toBe(false);
    expect(mockGlob('u_ex??????.LOG', s)).toBe(true);
  });

  it('ships parsers the mock preview reads', () => {
    expect(MOCK_PARSER_FILES.get(`${MOCK_PARSER_DIR}/log4net-csv.toml`)).toContain('version = "1.1.0"');
    const csv = mockPreviewParse({ toml: MOCK_PARSER_FILES.get(MOCK_IMPORT_PARSER_PATH)!, sample: '2025/11/08,00:00:00.013+01:00,INFO,Sync,Started\n2025/11/08,00:00:01.500+01:00,WARN,Sync,Slow', fileName: 'a.csv', limit: 10 });
    expect(csv).toMatchObject({ importer: 'csv', total: 2 });
    expect(csv.records[1]).toMatchObject({ level: 'Warning', message: 'Slow', ts: Date.UTC(2025, 10, 7, 23, 0, 1, 500) });
  });
});

describe('MockParserLibrary', () => {
  it('chooses a parser per source and lists the sources each one read', () => {
    const lib = new MockParserLibrary();
    expect(SOURCES.map((s) => lib.recordedId(s))).toEqual(['serilog-clef', 'serilog-text', 'csv', 'generic']);
    const list = lib.list(SOURCES);
    expect(list.stale).toBe(0);
    expect(list.parsers.find((p) => p.id === 'csv')).toMatchObject({ origin: 'builtin', enabled: true, sources: [{ id: 2, name: 'inventory-sync.csv' }] });
    expect(list.broken[0]?.file).toBe('legacy-gateway.toml');
    // Grouped by category, then by name.
    const cats = list.parsers.map((p) => p.category);
    expect([...cats].sort((a, b) => a.localeCompare(b))).toEqual(cats);
  });

  it('marks sources stale when their parser changes, is disabled or removed, until they are read again', () => {
    const lib = new MockParserLibrary();
    lib.list(SOURCES);
    lib.save(MOCK_PARSER_FILES.get(MOCK_IMPORT_PARSER_PATH)!);
    expect(lib.list(SOURCES).stale).toBe(1);
    expect(lib.reread(SOURCES)).toBe(1);
    expect(lib.recordedId(SOURCES[2]!)).toBe('inventory-csv');
    lib.setEnabled('inventory-csv', false);
    expect(lib.list(SOURCES).stale).toBe(1);
    lib.setEnabled('inventory-csv', true);
    expect(lib.list(SOURCES).stale).toBe(0);
    lib.remove('inventory-csv');
    expect(lib.list(SOURCES).stale).toBe(1);
    lib.reread(SOURCES);
    lib.setEnabled('generic', false);
    expect(lib.list(SOURCES).stale).toBe(1);
    expect(lib.reread(SOURCES)).toBe(1);
    expect(lib.recordedId(SOURCES[3]!)).toBeNull();
    expect(() => lib.setEnabled('nope', true)).toThrow(CmdError);
  });

  it('saves user parsers and overrides, refuses clashes and renames', () => {
    const lib = new MockParserLibrary();
    expect(() => lib.save('schema = 1')).toThrow('missing field `id`');
    lib.save(parser('mine'));
    expect(lib.get('mine').origin).toBe('user');
    let clash: unknown;
    try {
      lib.save(parser('mine', 'description = "again"'));
    } catch (e) {
      clash = e;
    }
    expect(clash).toMatchObject({ code: 'fileExists' });
    lib.save(parser('mine', 'description = "again"'), 'mine');
    lib.save(parser('mine', 'description = "forced"'), null, true);
    expect(lib.get('mine').toml).toContain('forced');
    lib.save(parser('renamed'), 'mine');
    expect(() => lib.get('mine')).toThrow('no parser "mine"');
    lib.save(MOCK_BUILTIN_PARSERS.get('nlog')!.replace('version = "1.0.0"', 'version = "2.0.0"'), 'nlog');
    expect(lib.get('nlog').origin).toBe('override');
    lib.remove('nlog');
    expect(lib.get('nlog').origin).toBe('builtin');
    expect(() => lib.remove('nlog')).toThrow('disable it instead');
    expect(() => lib.remove('nope')).toThrow(CmdError);
    expect(lib.validate(parser('ok'))).toEqual({ ok: true, error: null, id: 'ok', name: 'P ok', version: '1.0.0' });
    expect(lib.validate('x =')).toMatchObject({ ok: false, id: null });
  });

  it('duplicates with free ids', () => {
    const lib = new MockParserLibrary();
    expect(lib.duplicate('nlog')).toMatchObject({ id: 'nlog-copy', origin: 'user' });
    const second = lib.duplicate('nlog');
    expect(second.id).toBe('nlog-copy-2');
    expect(second.toml).toContain('name = "NLog default layout (copy)"');
    expect(() => lib.duplicate('nope')).toThrow(CmdError);
  });

  it('checks imports against the library and exports parser files', () => {
    const lib = new MockParserLibrary();
    expect(lib.importCheck(MOCK_IMPORT_PARSER_PATH)).toMatchObject({ relation: 'new', existing: null, id: 'inventory-csv' });
    const newer = lib.importCheck(`${MOCK_PARSER_DIR}/log4net-csv.toml`);
    expect(newer).toMatchObject({ relation: 'newer', version: '1.1.0', existing: { origin: 'builtin', version: '1.0.0' } });
    lib.save(newer.toml, 'log4net-csv', true);
    lib.exportTo('log4net-csv', '/out/l.toml');
    expect(lib.importCheck('/out/l.toml').relation).toBe('same');
    expect(() => lib.exportTo('log4net-csv', '/out/l.toml')).toThrow('already exists');
    lib.exportTo('log4net-csv', '/out/l.toml', true);
    lib.save(newer.toml.replace('1.1.0', '1.2.0'), 'log4net-csv', true);
    expect(lib.importCheck('/out/l.toml').relation).toBe('older');
    expect(() => lib.importCheck('/missing.toml')).toThrow('does not exist');
    lib.written.set('/out/bad.toml', 'schema = 1');
    expect(() => lib.importCheck('/out/bad.toml')).toThrow('missing field `id`');
    expect(() => lib.exportTo('nope', '/out/x.toml')).toThrow(CmdError);
  });

  it('drafts new parsers and changed copies for the format wizard', () => {
    const lib = new MockParserLibrary();
    const created = lib.draft({ fileName: 'My App.log', base: null, regex: '^(?P<message>.*)$', template: null, timestampFormat: null });
    expect(created).toMatchObject({ id: 'my-app.log', origin: 'user' });
    expect(mockParseParser(created.toml)).toMatchObject({ ok: true, parser: { kind: 'text', files: ['My App.log'] } });
    lib.save(created.toml);
    expect(lib.draft({ fileName: 'My App.log', base: null, regex: null, template: '{Timestamp} {Message}', timestampFormat: null }).id).toBe('my-app.log-2');
    expect(lib.draft({ fileName: '!!!', base: null, regex: null, template: null, timestampFormat: null }).id).toBe('custom');
    const fixed = lib.draft({ fileName: 'w.log', base: 'serilog-text', regex: null, template: null, timestampFormat: '%Y' });
    expect(fixed).toMatchObject({ id: 'serilog-text', origin: 'override' });
    expect(fixed.toml).toContain('timestamp_formats = [ "%Y" ]');
    expect(lib.draft({ fileName: 'w.log', base: 'my-app.log', regex: null, template: null, timestampFormat: '%Y' }).origin).toBe('user');
    expect(() => lib.draft({ fileName: 'w.log', base: 'nope', regex: null, template: null, timestampFormat: null })).toThrow(CmdError);
  });

  it('assigns parsers per source, even disabled ones, and back to automatic', () => {
    const lib = new MockParserLibrary();
    const worker = SOURCES[1]!;
    lib.setEnabled('nlog', false);
    lib.assign(worker, 'nlog');
    expect(lib.recordedId(worker)).toBe('nlog');
    expect(lib.list(SOURCES).stale).toBe(0);
    lib.assign(worker, null);
    expect(lib.recordedId(worker)).toBe('serilog-text');
    expect(() => lib.assign(worker, 'nope')).toThrow(CmdError);
    lib.setEnabled('generic', false);
    lib.assign(SOURCES[3]!, null);
    expect(lib.recordedId(SOURCES[3]!)).toBeNull();
  });
});
