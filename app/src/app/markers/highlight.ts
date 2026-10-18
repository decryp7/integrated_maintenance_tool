/** Pure proposals for the "Highlight lines" flow. */
import { RecordView } from '../core/api.types';
import { Condition } from '../core/profile.model';
import { escapeRegex } from '../context/regex-builder';

const GUID = '[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}';
const TOKEN = new RegExp(`(${GUID})|(0x[0-9a-fA-F]+)|(\\b[0-9a-fA-F]{12,}\\b)|(\\d+)`, 'g');

/** Escapes literal text; GUIDs become a GUID pattern, hex literals and long hex runs a hex pattern, digit runs `\d+`. */
export function generalizeText(text: string): string {
  let out = '';
  let last = 0;
  for (const m of text.matchAll(TOKEN)) {
    out += escapeRegex(text.slice(last, m.index));
    out += m[1] ? GUID : m[2] ? '0x[0-9a-fA-F]+' : m[3] ? '[0-9a-fA-F]{12,}' : '\\d+';
    last = m.index + m[0].length;
  }
  return out + escapeRegex(text.slice(last));
}

export interface HighlightProposal {
  conditions: Condition[];
  label: string;
  /** How the conditions were derived, for the dialog hint. */
  basis: 'selection' | 'template' | 'message';
}

const LABEL_MAX = 40;
const short = (s: string) => (s.length > LABEL_MAX ? `${s.slice(0, LABEL_MAX).trim()}…` : s);

/**
 * What to highlight for a record: the selected text as a regex when something is selected, else the
 * message template (literal), else the generalised message.
 */
export function proposeHighlight(record: RecordView, selection?: string | null): HighlightProposal {
  const picked = selection?.trim();
  if (picked) return { conditions: [{ field: '@message', op: 'matches', value: generalizeText(picked) }], label: short(picked), basis: 'selection' };
  if (record.template) return { conditions: [{ field: '@template', op: 'eq', value: record.template }], label: short(record.template), basis: 'template' };
  return { conditions: [{ field: '@message', op: 'matches', value: `^${generalizeText(record.message)}$` }], label: short(record.message), basis: 'message' };
}

const QL_TERM = /^([@\w.]+)\s*(=|!=|~|contains|!contains)\s*("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')$/i;
const QL_OPS: Record<string, Condition['op']> = { '=': 'eq', '!=': 'ne', '~': 'matches', contains: 'contains', '!contains': 'notContains' };

/** Converts `field op "value" and ...` to conditions when it is that simple (no or/not/pipes); else null. */
export function qlToConditions(ql: string): Condition[] | null {
  const text = ql.trim();
  if (!text || text.includes('|') || /[()]/.test(text.replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g, ''))) return null;
  const parts: string[] = [];
  // Split on top-level " and " outside quotes.
  const pieces = text.split(/("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')/);
  let buf = '';
  for (let i = 0; i < pieces.length; i++) {
    const piece = pieces[i] ?? '';
    if (i % 2) buf += piece;
    else {
      const bits = piece.split(/\s+and\s+/i);
      buf += bits[0] ?? '';
      for (let b = 1; b < bits.length; b++) {
        parts.push(buf);
        buf = bits[b] ?? '';
      }
    }
  }
  parts.push(buf);
  const out: Condition[] = [];
  for (const part of parts) {
    const m = QL_TERM.exec(part.trim());
    if (!m) return null;
    const [, name, opText, raw] = m;
    const op = opText === undefined ? undefined : QL_OPS[opText.toLowerCase()];
    if (name === undefined || raw === undefined || op === undefined) return null;
    const value = raw.slice(1, -1).replace(/\\(["'\\])/g, '$1');
    const field = ['level', 'message', 'exception', 'template', 'source'].includes(name.toLowerCase()) ? `@${name.toLowerCase()}` : name;
    out.push({ field, op, value });
  }
  return out;
}
