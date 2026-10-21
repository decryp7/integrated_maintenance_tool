import { parse } from 'smol-toml';
import { escapeRegex } from '../../context/regex-builder';
import { Level, ParsedPreview, ParsePreview, ParsePreviewRequest } from '../api.types';
import { toJsRegex } from '../profile.model';

/** Pattern for a .NET date/time format string ("yyyy-MM-dd HH:mm:ss.fff zzz"). Mock only. */
export function dateFormatPattern(format: string): string {
  return format
    .split(/(y+|M+|d+|H+|h+|m+|s+|f+|F+|z+|tt)/)
    .map((tok, i) => {
      if (i % 2 === 0) return escapeRegex(tok);
      switch (tok[0]) {
        case 'y':
          return tok.length >= 4 ? '\\d{4}' : '\\d{2}';
        case 'f':
        case 'F':
          return `\\d{${tok.length}}`;
        case 'z':
          return '[+-]\\d{2}(?::\\d{2})?';
        case 't':
          return '[AP]M';
        default:
          return tok.length === 1 ? '\\d{1,2}' : `\\d{${tok.length}}`;
      }
    })
    .join('');
}

/**
 * Converts a Serilog output template to a regex with named groups timestamp/level/message. Mock only: the
 * engine parses output templates itself; the UI never needs this. Returns null without Timestamp and Message.
 */
export function regexFromOutputTemplate(template: string): string | null {
  if (!/\{Timestamp[:}]/.test(template) || !/\{Message[:}]/.test(template)) return null;
  let out = '^';
  let last = 0;
  const holes = /\{(\w+)(?:[,:]([^}]*))?\}/g;
  for (let m = holes.exec(template); m; m = holes.exec(template)) {
    out += escapeRegex(template.slice(last, m.index));
    last = m.index + m[0].length;
    const [, name, fmt = ''] = m;
    switch (name) {
      case 'Timestamp':
        out += `(?P<timestamp>${fmt ? dateFormatPattern(fmt) : '.+?'})`;
        break;
      case 'Level':
        out += `(?P<level>${/^[uw]3$/.test(fmt) ? '[A-Za-z]{3}' : '[A-Za-z]+'})`;
        break;
      case 'Message':
        out += '(?P<message>.*)';
        break;
      case 'NewLine':
      case 'Exception':
      case 'Properties':
        break;
      default:
        out += `(?P<${name}>.*?)`;
    }
  }
  out += escapeRegex(template.slice(last));
  return out + '$';
}

const LEVEL_WORDS: [RegExp, Level][] = [
  [/^(vrb|verbose|trace)/i, 'Verbose'],
  [/^(dbg|debug)/i, 'Debug'],
  [/^(wrn|warn)/i, 'Warning'],
  [/^(err)/i, 'Error'],
  [/^(ftl|fatal|crit)/i, 'Fatal'],
];

export function mockLevel(text: string | undefined): Level {
  return LEVEL_WORDS.find(([re]) => re.test(text ?? ''))?.[1] ?? 'Information';
}

export function mockTimestamp(text: string | undefined): number | null {
  if (!text) return null;
  let iso = text.trim().replace(/^(\d{4})\/(\d{2})\/(\d{2})[ ,T]/, '$1-$2-$3T').replace(/^(\d{4}-\d{2}-\d{2}) (\d)/, '$1T$2').replace(/ ([+-]\d{2}:?\d{2})$/, '$1');
  if (!/(Z|[+-]\d{2}:?\d{2})$/.test(iso)) iso += 'Z';
  const t = Date.parse(iso);
  return Number.isNaN(t) ? null : t;
}

const empty = (error: string | null, skipped = 0): ParsePreview => ({ importer: null, records: [], total: 0, skipped, dropped: 0, error });

/** The parts of a parser TOML the mock preview reads (ADR 0026; the engine's `FormatCfg` is the reference). */
interface MockFormat {
  kind?: string;
  regexes?: string[];
  output_templates?: string[];
  json?: Partial<Record<JsonPart, string[]>>;
  csv?: { delimiter?: string; has_header?: boolean };
}

type JsonPart = 'timestamp' | 'level' | 'message' | 'exception';

const JSON_KEYS: Record<JsonPart, string[]> = {
  timestamp: ['@t', 'Timestamp', 'timestamp', '@timestamp', 'time'],
  level: ['@l', 'Level', 'level', 'severity'],
  message: ['@m', '@mt', 'RenderedMessage', 'MessageTemplate', 'Message', 'message', 'msg'],
  exception: ['@x', 'Exception', 'exception'],
};

type Reader = (line: string) => Omit<ParsedPreview, 'line'> | null;

function record(ts: string | undefined, level: string | undefined, message: string | undefined, exception: string | null = null): Omit<ParsedPreview, 'line'> {
  return { ts: mockTimestamp(ts), level: mockLevel(level), message: message ?? '', exception, fields: {}, markers: [] };
}

function jsonReader(keys: MockFormat['json']): Reader {
  const pick = (o: Record<string, unknown>, part: JsonPart): string | undefined => {
    for (const key of keys?.[part] ?? JSON_KEYS[part]) if (o[key] !== undefined && o[key] !== null) return String(o[key]);
    return undefined;
  };
  return (line) => {
    if (!line.trimStart().startsWith('{')) return null;
    try {
      const o = JSON.parse(line) as Record<string, unknown>;
      return record(pick(o, 'timestamp'), pick(o, 'level'), pick(o, 'message'), pick(o, 'exception') ?? null);
    } catch {
      return null;
    }
  };
}

/** CSV rows: header names when there is a header, else the date and time in the first columns and the message last. */
function csvReader(cfg: MockFormat['csv'], firstLine: string): Reader {
  const delimiter = cfg?.delimiter && cfg.delimiter !== 'auto' ? cfg.delimiter : ([',', ';', '\t', '|'].find((d) => firstLine.includes(d)) ?? ',');
  const header = cfg?.has_header === false ? null : firstLine.split(delimiter).map((h) => h.trim());
  const col = (re: RegExp) => header?.findIndex((h) => re.test(h)) ?? -1;
  const [ti, li, mi] = [col(/time|date/i), col(/level|severity/i), col(/message|msg/i)];
  return (line) => {
    if (header && line === firstLine) return null;
    const cells = line.split(delimiter);
    if (cells.length < 2) return null;
    if (header) return record(cells[ti], cells[li], cells[mi] ?? cells.at(-1));
    const dateTime = /^\d{4}[-/]\d{2}[-/]\d{2}$/.test(cells[0] ?? '') ? `${cells[0]} ${cells[1]}` : cells[0];
    const level = cells.find((c) => /^(trace|debug|info|information|warn|warning|error|fatal|verbose)$/i.test(c.trim()));
    return record(dateTime, level, cells.at(-1));
  };
}

/** Simplified stand-in for the engine's `preview_parse`: reads the sample with a candidate parser TOML (ADR 0026). */
export function mockPreviewParse(req: ParsePreviewRequest): ParsePreview {
  let format: MockFormat;
  try {
    format = (parse(req.toml) as { format?: MockFormat }).format ?? {};
  } catch (e) {
    return empty(e instanceof Error ? (e.message.split('\n')[0] ?? '') : String(e));
  }
  const lines = req.sample.split(/\r?\n/);
  const kind = format.kind ?? 'auto';
  const readers: Reader[] = [];
  if (kind === 'json' || kind === 'auto') readers.push(jsonReader(format.json));
  if (kind === 'csv') readers.push(csvReader(format.csv, lines.find((l) => l.trim()) ?? ''));
  const sources = [...(format.regexes ?? []), ...(format.output_templates ?? []).map(regexFromOutputTemplate).filter((r): r is string => !!r)];
  for (const source of sources) {
    let re: RegExp;
    try {
      re = toJsRegex(source);
    } catch (e) {
      return empty(`Invalid regex: ${e instanceof Error ? e.message : String(e)}`);
    }
    readers.push((line) => {
      const g = re.exec(line)?.groups;
      return g ? record(g['timestamp'], g['level'], g['message']) : null;
    });
  }
  const records: ParsedPreview[] = [];
  let skipped = 0;
  lines.forEach((line, i) => {
    if (!line.trim()) return;
    for (const read of readers) {
      const r = read(line);
      if (r) {
        records.push({ line: i + 1, ...r });
        return;
      }
    }
    const previous = records[records.length - 1];
    if (previous) previous.exception = previous.exception ? `${previous.exception}\n${line}` : line;
    else skipped++;
  });
  if (!records.length) return empty(null, skipped);
  const json = records.some((r) => lines[r.line - 1]?.trimStart().startsWith('{'));
  const importer = kind === 'auto' ? (json ? 'json' : 'text') : kind;
  return { importer, records: records.slice(0, Math.max(0, req.limit)), total: records.length, skipped, dropped: 0, error: null };
}

