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
  if (pos < line.length || entries[entries.length - 1]?.[0] !== 'message') out += '$';
  return out;
}
