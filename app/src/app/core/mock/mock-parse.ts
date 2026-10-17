import { parse } from 'smol-toml';
import { escapeRegex } from '../../context/regex-builder';
import { Level, ParsedPreview, ParsePreview, ParsePreviewRequest } from '../api.types';
import { ProfileModel, toJsRegex } from '../profile.model';

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
  let iso = text.trim().replace(/^(\d{4}-\d{2}-\d{2}) (\d)/, '$1T$2').replace(/ ([+-]\d{2}:?\d{2})$/, '$1');
  if (!/(Z|[+-]\d{2}:?\d{2})$/.test(iso)) iso += 'Z';
  const t = Date.parse(iso);
  return Number.isNaN(t) ? null : t;
}

const empty = (error: string | null, skipped = 0): ParsePreview => ({ importer: null, records: [], total: 0, skipped, dropped: 0, error });

/** Simplified stand-in for the engine's `preview_parse`: applies the profile's regexes and output templates line by line. */
export function mockPreviewParse(req: ParsePreviewRequest): ParsePreview {
  let model: ProfileModel;
  try {
    model = parse(req.toml) as ProfileModel;
  } catch (e) {
    return empty(e instanceof Error ? e.message.split('\n')[0] : String(e));
  }
  const sources = [...(model.parse?.regexes ?? []), ...(model.parse?.output_templates ?? []).map(regexFromOutputTemplate).filter((r): r is string => !!r)];
  const patterns: RegExp[] = [];
  for (const source of sources) {
    try {
      patterns.push(toJsRegex(source));
    } catch (e) {
      return empty(`Invalid regex: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  const lines = req.sample.split(/\r?\n/);
  const records: ParsedPreview[] = [];
  let skipped = 0;
  lines.forEach((line, i) => {
    if (!line.trim()) return;
    for (const re of patterns) {
      const g = re.exec(line)?.groups;
      if (g) {
        records.push({ line: i + 1, ts: mockTimestamp(g['timestamp']), level: mockLevel(g['level']), message: g['message'] ?? '', exception: null, fields: {}, markers: [] });
        return;
      }
    }
    const previous = records[records.length - 1];
    if (previous) previous.exception = previous.exception ? `${previous.exception}\n${line}` : line;
    else skipped++;
  });
  if (!records.length) return empty(null, skipped);
  return { importer: 'text', records: records.slice(0, Math.max(0, req.limit)), total: records.length, skipped, dropped: 0, error: null };
}
