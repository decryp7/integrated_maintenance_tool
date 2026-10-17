import { toJsRegex } from '../core/profile.model';
import { escapeRegex } from '../context/regex-builder';

export type FormatPart = 'timestamp' | 'level' | 'message';

/** Character range (UTF-16 offsets) the user marked on the sample line. */
export interface Mark {
  start: number;
  end: number;
}

export type Marks = Partial<Record<FormatPart, Mark>>;

/** Pattern for a timestamp as written: digit runs keep their length, other characters are literal. */
export function timestampPattern(text: string): string {
  return text
    .split(/(\d+)/)
    .map((part, i) => (i % 2 ? `\\d{${part.length}}` : escapeRegex(part)))
    .join('');
}

function levelPattern(text: string): string {
  return /^[A-Za-z]+$/.test(text) ? '[A-Za-z]+' : '\\w+';
}

/**
 * Builds a regex (Rust syntax, named groups timestamp/level/message) from marks on a sample line.
 * Text outside the marks is kept literally, the message runs to the end of the line.
 * Returns null unless timestamp and message are marked and the marks do not overlap.
 */
export function regexFromMarks(line: string, marks: Marks): string | null {
  const entries = (Object.entries(marks) as [FormatPart, Mark][]).filter(([, m]) => m.end > m.start).sort((a, b) => a[1].start - b[1].start);
  if (!marks.timestamp || !marks.message || entries.length < 2) return null;
  let out = '^';
  let pos = 0;
  for (const [part, m] of entries) {
    if (m.start < pos || m.end > line.length) return null;
    out += escapeRegex(line.slice(pos, m.start));
    const text = line.slice(m.start, m.end);
    const group = part === 'timestamp' ? timestampPattern(text) : part === 'level' ? levelPattern(text) : m.end === line.length ? '.*' : '.+?';
    out += `(?P<${part}>${group})`;
    pos = m.end;
  }
  out += escapeRegex(line.slice(pos));
  if (pos < line.length || entries[entries.length - 1][0] !== 'message') out += '$';
  return out;
}

/** Pattern for a .NET date/time format string ("yyyy-MM-dd HH:mm:ss.fff zzz"). */
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
 * Converts a Serilog output template ("{Timestamp:yyyy-MM-dd HH:mm:ss} [{Level:u3}] {Message:lj}{NewLine}{Exception}")
 * to a regex with the same named groups, for the live preview. Returns null for a template without
 * Timestamp and Message.
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

export interface ParsedLine {
  line: string;
  matched: boolean;
  timestamp?: string;
  level?: string;
  message?: string;
}

/** Applies the regex to each line (client side, same named groups the engine uses). */
export function parseLines(lines: readonly string[], regex: string): { rows: ParsedLine[]; error: string | null } {
  let re: RegExp;
  try {
    re = toJsRegex(regex);
  } catch (e) {
    return { rows: lines.map((line) => ({ line, matched: false })), error: e instanceof Error ? e.message : String(e) };
  }
  return {
    rows: lines.map((line) => {
      const m = re.exec(line);
      if (!m) return { line, matched: false };
      const g = m.groups ?? {};
      return { line, matched: true, timestamp: g['timestamp'], level: g['level'], message: g['message'] };
    }),
    error: null,
  };
}

/** How many of the rows matched. Continuation lines (stack traces) legitimately do not. */
export function matchedCount(rows: readonly ParsedLine[]): number {
  return rows.filter((r) => r.matched).length;
}
