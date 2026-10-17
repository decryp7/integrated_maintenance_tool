import { FILE_EXISTS_SUFFIX } from '../../core/api.types';
import {
  ChangeKind,
  CompareOptions,
  DiffCounts,
  DiffExported,
  DiffExportFormat,
  DiffFormats,
  DiffOptions,
  DiffPage,
  DiffRow,
  SnapshotDiff,
  ValueDto,
  ValueKind,
} from '../../core/config-diff.types';
import { ConfigDiffBackend } from '../config-diff-api';

/**
 * Browser-only stand-in for the engine's configuration diff (ng serve, UI tests): a canned diff of
 * two `appsettings.json` files with the same paging, filtering and folding contract. Paths containing
 * `missing` or `broken` fail like unreadable files. Not a reference for behaviour.
 */

export const MOCK_DEFAULT_IGNORE = ['*InstallDate*', '*InstallTime*', '*LastWriteTime*', '*LastModified*', '*LastUpdated*', '*LastBootUpTime*', '*Timestamp*'];

interface Entry {
  path: string;
  kind: ChangeKind;
  old: ValueDto | null;
  new: ValueDto | null;
}

const v = (kind: ValueKind, text: string): ValueDto => ({ kind, text, clipped: false });
const S = v('section', '');

function entry(path: string, kind: ChangeKind, old: ValueDto | null, nu: ValueDto | null = old): Entry {
  return { path, kind, old: kind === 'added' ? null : old, new: kind === 'removed' ? null : nu };
}

/** The demo diff, in tree order (parents first). */
export function demoEntries(): Entry[] {
  const sources: Entry[] = [];
  for (let i = 0; i < 200; i++) {
    const name = `Logging/Sources/Source${String(i).padStart(3, '0')}`;
    sources.push(i % 70 === 5 ? entry(name, 'changed', v('string', 'Information'), v('string', 'Debug')) : entry(name, 'unchanged', v('string', 'Information')));
  }
  return [
    entry('ConnectionStrings', 'unchanged', S),
    entry('ConnectionStrings/Audit', 'unchanged', v('string', 'Server=audit01;Database=Audit')),
    entry('ConnectionStrings/Orders', 'changed', v('string', 'Server=db01;Database=Orders'), v('string', 'Server=db02;Database=Orders')),
    entry('Features', 'unchanged', S),
    entry('Features/Legacy', 'removed', v('bool', 'true')),
    entry('Features/NewCheckout', 'changed', v('bool', 'false'), v('bool', 'true')),
    entry('InstallDate', 'changed', v('string', '2026-10-01'), v('string', '2026-10-15')),
    entry('Logging', 'unchanged', S),
    entry('Logging/LogLevel', 'unchanged', S),
    entry('Logging/LogLevel/Default', 'unchanged', v('string', 'Information')),
    entry('Logging/LogLevel/Microsoft', 'changed', v('string', 'Warning'), v('string', 'Error')),
    entry('Logging/Sources', 'unchanged', S),
    ...sources,
    entry('Servers', 'unchanged', S),
    entry('Servers[0]', 'unchanged', S),
    entry('Servers[0]/host', 'unchanged', v('string', 'web01')),
    entry('Servers[0]/port', 'changed', v('integer', '8080'), v('integer', '8443')),
    entry('Servers[1]', 'added', null, S),
    entry('Servers[1]/host', 'added', null, v('string', 'web02')),
    entry('Servers[1]/port', 'added', null, v('integer', '8443')),
    entry('Timeout', 'added', null, v('integer', '30')),
  ];
}

const depthOf = (path: string) => path.split('/').length - 1 + (path.match(/\[\d+\]/g)?.length ?? 0);
const nameOf = (path: string) => {
  const last = path.split('/').at(-1) ?? path;
  const idx = /(\[\d+\])$/.exec(last);
  return idx && last !== idx[1] ? idx[1] : last;
};
const isUnder = (path: string, parent: string) => path.startsWith(`${parent}/`) || path.startsWith(`${parent}[`);

/** `*x*` globs and `re:` regexes, case-insensitive, over the whole path. */
function ignoreMatcher(patterns: string[]): (path: string) => boolean {
  const res = patterns
    .map((p) => p.trim())
    .filter(Boolean)
    .map((p) => (p.startsWith('re:') ? new RegExp(p.slice(3), 'i') : new RegExp(`^${p.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.')}$`, 'i')));
  return (path) => res.some((r) => r.test(path));
}

/** Applies ignore patterns, kinds, search, folding and paging like the engine does. */
export function mockPage(entries: Entry[], options: DiffOptions): Omit<DiffPage, 'left' | 'right' | 'caseInsensitive' | 'truncated'> {
  const ignored = ignoreMatcher(options.compare.ignore);
  const counts: DiffCounts = { added: 0, removed: 0, changed: 0, unchanged: 0, ignored: 0 };
  const kept: Entry[] = [];
  let skipUnder: string | null = null;
  for (const e of entries) {
    if (skipUnder && isUnder(e.path, skipUnder)) {
      counts.ignored++;
      continue;
    }
    skipUnder = null;
    if (ignored(e.path)) {
      counts.ignored++;
      skipUnder = e.path;
      continue;
    }
    counts[e.kind]++;
    kept.push(e);
  }
  const changed = (p: string) => kept.some((e) => e.kind !== 'unchanged' && isUnder(e.path, p));
  const search = options.view.search.trim().toLowerCase();
  const matches = (e: Entry) =>
    options.view.kinds.includes(e.kind) && (!search || [e.path, e.old?.text ?? '', e.new?.text ?? ''].some((s) => s.toLowerCase().includes(search)));
  const matched = new Set(kept.filter(matches).map((e) => e.path));
  const shown = kept.filter((e) => matched.has(e.path) || [...matched].some((m) => isUnder(m, e.path)));
  const toggled = new Set(options.view.toggled);
  const rows: DiffRow[] = [];
  let foldUnder: string | null = null;
  shown.forEach((e, i) => {
    if (foldUnder && isUnder(e.path, foldUnder)) return;
    foldUnder = null;
    const expandable = !!shown[i + 1] && isUnder(shown[i + 1].path, e.path);
    const hasChanges = changed(e.path);
    const byDefault = !!search || options.view.expand === 'all' || hasChanges;
    const expanded = expandable && byDefault !== toggled.has(e.path);
    if (expandable && !expanded) foldUnder = e.path;
    rows.push({ path: e.path, name: nameOf(e.path), depth: depthOf(e.path), kind: e.kind, old: e.old, new: e.new, hasChanges, expandable, expanded, context: !matched.has(e.path) });
  });
  const offset = Math.min(options.view.offset, rows.length);
  return { counts, total: rows.length, offset, rows: rows.slice(offset, offset + Math.max(1, Math.min(options.view.limit, 5000))) };
}

function failure(path: string, side: string): string | null {
  if (path.includes('missing')) return `${side} file ${path}: No such file or directory (os error 2)`;
  if (path.includes('broken')) return `${side} file: JSON: expected value at line 1 column 1`;
  return null;
}

export class MockConfigDiffBackend extends ConfigDiffBackend {
  readonly calls: string[] = [];

  async formats(): Promise<DiffFormats> {
    this.calls.push('formats');
    return {
      formats: [
        { id: 'json', name: 'JSON', extensions: ['json'] },
        { id: 'xml', name: 'XML', extensions: ['xml', 'config'] },
        { id: 'toml', name: 'TOML', extensions: ['toml'] },
        { id: 'yaml', name: 'YAML', extensions: ['yaml', 'yml'] },
        { id: 'ini', name: 'INI', extensions: ['ini', 'cfg', 'conf', 'inf', 'properties'] },
        { id: 'registry', name: 'Registry export', extensions: ['reg'] },
        { id: 'msinfo', name: 'System information (msinfo32)', extensions: ['nfo'] },
      ],
      defaultIgnore: [...MOCK_DEFAULT_IGNORE],
    };
  }

  async diff(leftPath: string, rightPath: string, options: DiffOptions): Promise<DiffPage> {
    this.calls.push(`diff:${options.view.offset}`);
    const error = failure(leftPath, 'left') ?? failure(rightPath, 'right');
    if (error) throw error;
    if (options.compare.ignore.some((p) => p.startsWith('re:('))) throw 'ignore pattern "re:(": regex parse error';
    const side = (path: string) => ({ path, format: 'json', nodes: 222, truncated: false });
    return { left: side(leftPath), right: side(rightPath), caseInsensitive: options.compare.caseInsensitive, truncated: false, ...mockPage(demoEntries(), options) };
  }

  async snapshots(leftDir: string, rightDir: string, compare: CompareOptions): Promise<SnapshotDiff> {
    this.calls.push(`snapshots:${compare.ignore.length}`);
    if (leftDir.includes('missing')) throw `left folder ${leftDir}: No such file or directory (os error 2)`;
    const counts = (changed: number): DiffCounts => ({ added: 0, removed: 0, changed, unchanged: 20, ignored: 1 });
    const file = (relPath: string, status: SnapshotDiff['files'][number]['status'], format: string | null, changed = 0, error: string | null = null) => ({
      relPath,
      leftPath: status === 'added' ? null : `${leftDir}/${relPath}`,
      rightPath: status === 'removed' ? null : `${rightDir}/${relPath}`,
      status,
      format,
      counts: status === 'changed' || status === 'unchanged' ? counts(changed) : null,
      error,
    });
    return {
      left: leftDir,
      right: rightDir,
      truncated: false,
      files: [
        file('app/appsettings.json', 'changed', 'json', 3),
        file('registry/services.reg', 'changed', 'registry', 2),
        file('system/msinfo.nfo', 'unchanged', 'msinfo'),
        file('app/web.config', 'removed', null),
        file('app/new.yaml', 'added', null),
        file('app/broken.json', 'unreadable', null, 0, 'left: JSON: EOF while parsing an object at line 1 column 1'),
      ],
    };
  }

  async exportDiff(leftPath: string, rightPath: string, options: DiffOptions, format: DiffExportFormat, path: string, overwrite = false): Promise<DiffExported> {
    if (!overwrite && this.calls.some((c) => c.startsWith('export:') && c.endsWith(`:${path}`))) throw new Error(`${path}${FILE_EXISTS_SUFFIX}`);
    this.calls.push(`export:${format}:${path}`);
    const all = mockPage(demoEntries(), { ...options, view: { ...options.view, expand: 'all', toggled: [], offset: 0, limit: 5000 } });
    return { path, rows: all.rows.filter((r) => !r.context).length };
  }
}
