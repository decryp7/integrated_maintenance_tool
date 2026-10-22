import { parse, stringify, TomlError } from 'smol-toml';
import {
  BrokenParserDto,
  FILE_EXISTS_SUFFIX,
  ParserDocDto,
  ParserDraftRequest,
  ParserEntryDto,
  ParserErrorDto,
  ParserImportCheckDto,
  ParserLibraryDto,
  ParserOrigin,
  ParserRelation,
  ParserValidationDto,
} from '../api.types';
import { CmdError } from '../cmd-error';
import { MOCK_BROKEN_PARSER, MOCK_BUILTIN_PARSERS, MOCK_PARSER_FILES } from './mock-parsers-data';

/** What the mock library needs to know about a source to choose its parser. */
export interface MockParserSource {
  id: number;
  path: string;
  name: string;
  /** The mock importer that read it ("json", "text", "csv", "registry"). */
  importer: string;
}

/** A parser file as the mock understands it (a small part of the engine's schema, ADR 0026). */
export interface MockParser {
  id: string;
  name: string;
  version: string;
  description: string;
  category: string;
  kind: string;
  priority: number;
  files: string[] | null;
}

const ID = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const SEMVER = /^(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$/;

/** 1-based line of the first `key = …` line, or null. */
function lineOf(toml: string, key: string): number | null {
  const i = toml.split(/\r?\n/).findIndex((l) => new RegExp(`^\\s*${key}\\s*=`).test(l));
  return i < 0 ? null : i + 1;
}

const fail = (message: string, line: number | null = null, column: number | null = null): { ok: false; error: ParserErrorDto } => ({ ok: false, error: { message, line, column } });

/**
 * Reads a parser file the way the mock needs it: TOML syntax with line and column, `schema = 1`, the id pattern,
 * a name and a semver version. The engine's checks (unknown keys, limits, regexes) are the reference.
 */
export function mockParseParser(toml: string): { ok: true; parser: MockParser } | { ok: false; error: ParserErrorDto } {
  let doc: Record<string, unknown>;
  try {
    doc = parse(toml) as Record<string, unknown>;
  } catch (e) {
    if (e instanceof TomlError) return fail(e.message.split('\n')[0] ?? 'invalid TOML', e.line, e.column);
    return fail(String(e));
  }
  const schema = doc['schema'];
  if (schema === undefined) return fail('missing field `schema`', 1, 1);
  if (schema !== 1) return fail(`this parser was written for parser schema ${String(schema)}; this IMT reads schema 1 — update IMT`, lineOf(toml, 'schema'), 1);
  const id = doc['id'];
  if (typeof id !== 'string') return fail('missing field `id`', 1, 1);
  if (!ID.test(id)) return fail(`invalid id "${id}": use lowercase letters, digits, ".", "_" and "-" (at most 64)`, lineOf(toml, 'id'), 1);
  const name = doc['name'];
  if (typeof name !== 'string' || !name.trim()) return fail('missing field `name`', 1, 1);
  const version = doc['version'];
  if (typeof version !== 'string') return fail('missing field `version`', 1, 1);
  if (!SEMVER.test(version)) return fail(`invalid version "${version}": use semver, e.g. 1.0.0`, lineOf(toml, 'version'), 1);
  const match = (doc['match'] ?? {}) as Record<string, unknown>;
  const format = (doc['format'] ?? {}) as Record<string, unknown>;
  const files = Array.isArray(match['files']) ? (match['files'] as unknown[]).map(String) : null;
  return {
    ok: true,
    parser: {
      id,
      name,
      version,
      description: typeof doc['description'] === 'string' ? doc['description'] : '',
      category: typeof doc['category'] === 'string' ? doc['category'] : 'Custom',
      kind: typeof format['kind'] === 'string' ? format['kind'] : 'auto',
      priority: typeof match['priority'] === 'number' ? match['priority'] : 0,
      files,
    },
  };
}

/** Compares two semver versions (pre-release suffixes ignored). */
export function compareVersions(a: string, b: string): number {
  const pa = SEMVER.exec(a)?.slice(1, 4).map(Number) ?? [0, 0, 0];
  const pb = SEMVER.exec(b)?.slice(1, 4).map(Number) ?? [0, 0, 0];
  for (let i = 0; i < 3; i++) if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pa[i] ?? 0) - (pb[i] ?? 0);
  return 0;
}

/** The next patch version (`1.2.3` → `1.2.4`); a version that is not semver becomes `1.0.1`. */
export function bumpPatch(version: string): string {
  const m = SEMVER.exec(version);
  return m ? `${m[1]}.${m[2]}.${Number(m[3]) + 1}` : '1.0.1';
}

/** A quick fix's TOML fragment (`[format]` keys), or a refusal with the reason, like the engine's. */
export function parseFragment(fragment: string): Record<string, unknown> {
  try {
    return parse(fragment) as Record<string, unknown>;
  } catch (e) {
    const reason = e instanceof TomlError ? e.message.split('\n')[0] : String(e);
    throw new CmdError({ code: 'invalid', message: `the fix is not a valid parser change: ${reason}`, retryable: false });
  }
}

/** Merges `over` into `base` like the engine's profile merge: tables merge, other values (arrays too) replace. */
export function mergeTables(base: Record<string, unknown>, over: Record<string, unknown>): void {
  const isTable = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v) && !(v instanceof Date);
  for (const [key, value] of Object.entries(over)) {
    const current = base[key];
    if (isTable(current) && isTable(value)) mergeTables(current, value);
    else base[key] = value;
  }
}

/** FNV-1a of the text, as hex: the mock's content hash. */
export function mockHash(text: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 0x01000193) >>> 0;
  return h.toString(16).padStart(8, '0');
}

/** A `files` glob against a file name or path (`*` within a segment, `**` across). */
export function mockGlob(glob: string, source: Pick<MockParserSource, 'path' | 'name'>): boolean {
  const re = new RegExp(
    `^${glob
      .split('**')
      .map((part) => part.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '[^/\\\\]*').replace(/\?/g, '[^/\\\\]'))
      .join('.*')}$`,
    'i',
  );
  return re.test(source.name) || re.test(source.path.replace(/^[\\/]+/, ''));
}

const KIND_IMPORTERS: Record<string, string> = { json: 'json', csv: 'csv', text: 'text' };

const invalid = (error: ParserErrorDto): CmdError =>
  new CmdError({ code: 'invalid', message: error.message, retryable: false, ...(error.line !== null ? { line: error.line } : {}) });
const notFound = (id: string): CmdError => new CmdError({ code: 'notFound', message: `there is no parser "${id}" in the library`, retryable: false });

/**
 * The browser build's parser library (ADR 0026): built-ins, user parsers and overrides, enabled flags, the parser
 * each source was read with and "Parse with…" choices. Plausible, never the reference: the engine is.
 */
export class MockParserLibrary {
  private readonly builtins = new Map(MOCK_BUILTIN_PARSERS);
  private readonly user = new Map<string, string>();
  private readonly disabled = new Set<string>();
  private readonly broken: BrokenParserDto[] = [structuredClone(MOCK_BROKEN_PARSER)];
  /** Files "written" by export (path → TOML); import reads them too. */
  readonly written = new Map<string, string>();
  /** The parser each source was read with (id and content hash). */
  private readonly recorded = new Map<number, { id: string; hash: string }>();
  /** "Parse with…" choices by source id. */
  private readonly assigned = new Map<number, string>();

  private toml(id: string): string | undefined {
    return this.user.get(id) ?? this.builtins.get(id);
  }

  private origin(id: string): ParserOrigin {
    return this.user.has(id) ? (this.builtins.has(id) ? 'override' : 'user') : 'builtin';
  }

  private parsed(id: string): MockParser | null {
    const toml = this.toml(id);
    if (toml === undefined) return null;
    const r = mockParseParser(toml);
    return r.ok ? r.parser : null;
  }

  private ids(): string[] {
    return [...new Set([...this.builtins.keys(), ...this.user.keys()])];
  }

  /** How well a parser reads a source, or null when it cannot (file globs, then the format's kind). */
  private score(p: MockParser, s: MockParserSource): number | null {
    let score = 0;
    if (p.files) {
      if (!p.files.some((g) => mockGlob(g, s))) return null;
      score += 20;
    }
    if (p.kind === 'auto') return score + 10;
    if ((KIND_IMPORTERS[p.kind] ?? p.kind) !== s.importer) return null;
    return score + 50;
  }

  /** The parser the library would read a source with now: its "Parse with…" choice, else the best enabled one. */
  choose(s: MockParserSource): string | null {
    const forced = this.assigned.get(s.id);
    if (forced !== undefined && this.toml(forced) !== undefined) return forced;
    const ranked = this.ids()
      .filter((id) => !this.disabled.has(id))
      .map((id) => ({ id, p: this.parsed(id) }))
      .map(({ id, p }) => ({ id, p, score: p ? this.score(p, s) : null }))
      .filter((c): c is { id: string; p: MockParser; score: number } => c.score !== null)
      .sort((a, b) => b.score - a.score || b.p.priority - a.p.priority || Number(this.user.has(b.id)) - Number(this.user.has(a.id)) || a.id.localeCompare(b.id));
    return ranked[0]?.id ?? null;
  }

  /** The parser the library would read a sample with (`preview_parse` without a candidate parser). */
  chooseFor(fileName: string, sample: string): string | null {
    const head = sample.trimStart();
    const importer = head.startsWith('{') ? 'json' : /\.csv$/i.test(fileName) ? 'csv' : 'text';
    return this.choose({ id: -1, path: fileName, name: fileName.split(/[\\/]/).pop() ?? fileName, importer });
  }

  private current(s: MockParserSource): { id: string; hash: string } | null {
    const id = this.choose(s);
    return id === null ? null : { id, hash: mockHash(this.toml(id) ?? '') };
  }

  /** Sources seen for the first time were read with the library as it is now. */
  private ensureRecorded(sources: readonly MockParserSource[]): void {
    for (const s of sources) {
      if (this.recorded.has(s.id)) continue;
      const now = this.current(s);
      if (now) this.recorded.set(s.id, now);
    }
  }

  /** Id of the parser that read the source (`SourceInfo.parser`). */
  recordedId(s: MockParserSource): string | null {
    this.ensureRecorded([s]);
    return this.recorded.get(s.id)?.id ?? null;
  }

  private isStale(s: MockParserSource): boolean {
    const was = this.recorded.get(s.id);
    const now = this.current(s);
    return !!was && (!now || was.id !== now.id || was.hash !== now.hash);
  }

  list(sources: readonly MockParserSource[]): ParserLibraryDto {
    this.ensureRecorded(sources);
    const parsers: ParserEntryDto[] = [];
    for (const id of this.ids()) {
      const p = this.parsed(id);
      if (!p) continue;
      parsers.push({
        id,
        name: p.name,
        version: p.version,
        description: p.description,
        category: p.category,
        kind: p.kind,
        priority: p.priority,
        origin: this.origin(id),
        enabled: !this.disabled.has(id),
        hash: mockHash(this.toml(id) ?? ''),
        sources: sources.filter((s) => this.recorded.get(s.id)?.id === id).map((s) => ({ id: s.id, name: s.name, chosen: this.assigned.get(s.id) === id })),
      });
    }
    parsers.sort((a, b) => a.category.localeCompare(b.category) || a.name.localeCompare(b.name));
    return { parsers, broken: this.broken.map((b) => structuredClone(b)), stale: sources.filter((s) => this.isStale(s)).length };
  }

  get(id: string): ParserDocDto {
    const toml = this.toml(id);
    if (toml === undefined) throw notFound(id);
    return { id, origin: this.origin(id), toml };
  }

  validate(toml: string): ParserValidationDto {
    const r = mockParseParser(toml);
    return r.ok ? { ok: true, error: null, id: r.parser.id, name: r.parser.name, version: r.parser.version } : { ok: false, error: r.error, id: null, name: null, version: null };
  }

  save(toml: string, previousId: string | null = null, overwrite = false): void {
    const r = mockParseParser(toml);
    if (!r.ok) throw invalid(r.error);
    const id = r.parser.id;
    if (this.user.has(id) && previousId !== id && !overwrite) {
      throw new CmdError({ code: 'fileExists', message: `a parser with the id "${id}" is already in the library; confirm to replace it`, retryable: false });
    }
    // Renamed while editing: the user parser under its old id goes (a built-in stays).
    if (previousId && previousId !== id) this.user.delete(previousId);
    this.user.set(id, toml);
  }

  duplicate(id: string): ParserDocDto {
    const toml = this.toml(id);
    const p = this.parsed(id);
    if (toml === undefined || !p) throw notFound(id);
    let copy = `${id}-copy`;
    for (let n = 2; this.toml(copy) !== undefined; n++) copy = `${id}-copy-${n}`;
    const text = toml.replace(/^(\s*id\s*=\s*).*$/m, `$1"${copy}"`).replace(/^(\s*name\s*=\s*).*$/m, `$1${JSON.stringify(`${p.name} (copy)`)}`);
    this.user.set(copy, text);
    return { id: copy, origin: 'user', toml: text };
  }

  remove(id: string): void {
    if (!this.user.has(id)) {
      if (this.builtins.has(id)) throw new CmdError({ code: 'invalid', message: `"${id}" is built in: disable it instead`, retryable: false });
      throw notFound(id);
    }
    this.user.delete(id);
  }

  setEnabled(id: string, enabled: boolean): void {
    if (this.toml(id) === undefined) throw notFound(id);
    if (enabled) this.disabled.delete(id);
    else this.disabled.add(id);
  }

  importCheck(path: string): ParserImportCheckDto {
    const toml = MOCK_PARSER_FILES.get(path) ?? this.written.get(path);
    if (toml === undefined) throw new CmdError({ code: 'notFound', message: `${path} does not exist`, retryable: false });
    const r = mockParseParser(toml);
    if (!r.ok) throw invalid(r.error);
    const { id, name, version } = r.parser;
    const existing = this.parsed(id);
    let relation: ParserRelation = 'new';
    if (existing) {
      const c = compareVersions(version, existing.version);
      relation = c > 0 ? 'newer' : c === 0 ? 'same' : 'older';
    }
    return { toml, id, name, version, relation, existing: existing ? { origin: this.origin(id), version: existing.version, toml: this.toml(id) ?? '' } : null };
  }

  exportTo(id: string, path: string, overwrite = false): void {
    const toml = this.toml(id);
    if (toml === undefined) throw notFound(id);
    if (!overwrite && (this.written.has(path) || MOCK_PARSER_FILES.has(path))) throw new Error(`${path}${FILE_EXISTS_SUFFIX}`);
    this.written.set(path, toml);
  }

  /** The format wizard's candidate (not saved): a copy of `base` with additions, or a new parser for the file. */
  draft(req: ParserDraftRequest): ParserDocDto {
    const add = (format: Record<string, unknown>) => {
      const prepend = (key: string, value: string | null) => {
        if (!value) return;
        const list = Array.isArray(format[key]) ? (format[key] as unknown[]).map(String) : [];
        format[key] = [value, ...list.filter((v) => v !== value)];
      };
      prepend('timestamp_formats', req.timestampFormat);
      prepend('output_templates', req.template);
      prepend('regexes', req.regex);
    };
    const fragment = req.parserOverride?.trim() ? req.parserOverride : null;
    if (req.base) {
      const toml = this.toml(req.base);
      if (toml === undefined) throw notFound(req.base);
      const doc = parse(toml) as Record<string, unknown>;
      // A quick fix's `[format]` fragment is merged over the parser (tables merge, other values replace); it keeps its id.
      if (fragment !== null) {
        mergeTables(doc, parseFragment(fragment));
        doc['id'] = req.base;
        doc['version'] = bumpPatch(String((parse(toml) as Record<string, unknown>)['version'] ?? '1.0.0'));
      }
      const format = ((doc['format'] as Record<string, unknown> | undefined) ?? {}) as Record<string, unknown>;
      add(format);
      doc['format'] = format;
      return { id: req.base, origin: this.builtins.has(req.base) ? 'override' : 'user', toml: stringify(doc) + '\n' };
    }
    if (fragment !== null) throw new CmdError({ code: 'invalid', message: 'a parser change needs the parser it changes', retryable: false });
    const slug =
      req.fileName
        .toLowerCase()
        .replace(/[^a-z0-9._-]+/g, '-')
        .replace(/^[^a-z0-9]+/, '')
        .slice(0, 56) || 'custom';
    let id = slug;
    for (let n = 2; this.toml(id) !== undefined; n++) id = `${slug}-${n}`;
    const format: Record<string, unknown> = { kind: 'text' };
    add(format);
    const doc = { schema: 1, id, name: `Format of ${req.fileName}`, version: '1.0.0', description: `Taught with the format wizard from ${req.fileName}.`, category: 'Custom', match: { files: [req.fileName] }, format };
    return { id, origin: 'user', toml: stringify(doc) + '\n' };
  }

  /** `parsers_reread`: records the current choice for every source; returns how many changed. */
  reread(sources: readonly MockParserSource[]): number {
    let changed = 0;
    for (const s of sources) {
      if (this.isStale(s)) changed++;
      const now = this.current(s);
      if (now) this.recorded.set(s.id, now);
      else this.recorded.delete(s.id);
    }
    return changed;
  }

  /** `parser_assign`: "Parse with…" one source (null: the automatic choice); the source is read again with it. */
  assign(source: MockParserSource, parserId: string | null): void {
    if (parserId === null) this.assigned.delete(source.id);
    else if (this.toml(parserId) === undefined) throw notFound(parserId);
    else this.assigned.set(source.id, parserId);
    const now = this.current(source);
    if (now) this.recorded.set(source.id, now);
    else this.recorded.delete(source.id);
  }
}
