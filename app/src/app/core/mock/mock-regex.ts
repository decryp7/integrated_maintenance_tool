import { ExplainNode, GroupKind, InferredGroup, InferredRegex } from '../api.types';
import { toJsRegex } from '../profile.model';

/*
 * Canned stand-ins for the engine's `infer_regex` / `explain_regex` (crates/imt-core/src/regex_assist.rs),
 * for browser development and UI tests only. They return plausible output of the right shape; they are
 * NOT the reference behaviour (the engine names groups after neighbouring words, recognises more value
 * kinds and explains the full regex syntax tree).
 */

const enc = new TextEncoder();
/** Byte offset of UTF-16 index `i` in `s` (the engine reports UTF-8 byte offsets). */
const byteOffset = (s: string, i: number) => enc.encode(s.slice(0, i)).length;
const escape = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** Splits a pattern into the pieces the mock explains, one flat outline line each. */
export function mockExplainRegex(pattern: string): ExplainNode[] {
  const nodes: ExplainNode[] = [];
  const re = /\(\?P<(\w+)>|\\d\+|\\[dwsDWS][+*?]?|\[[^\]]*\](?:\{\d+(?:,\d*)?\}|[+*?])?|\\.|\.[+*?]|[()|^$]|[^\\()[\]|^$.]+/g;
  for (const m of pattern.matchAll(re)) {
    nodes.push({ depth: 0, text: describe(m[0], m[1]), start: byteOffset(pattern, m.index), end: byteOffset(pattern, m.index + m[0].length) });
  }
  return nodes;
}

const FIXED: Record<string, string> = {
  '\\d+': 'One or more digits',
  '(': 'Start group',
  ')': 'End group',
  '|': 'Or',
  '^': 'Start of text',
  $: 'End of text',
  '.+': 'Any characters',
  '.*': 'Any characters',
};

function describe(token: string, group: string | undefined): string {
  if (group) return `Start group "${group}"`;
  if (FIXED[token]) return FIXED[token];
  if (token.startsWith('\\')) return `Escaped "${token.slice(1)}"`;
  if (token.startsWith('[')) return `One of the characters ${token}`;
  return `Literal "${token}"`;
}

function kindOf(v: string): GroupKind {
  if (/^-?\d+$/.test(v)) return 'number';
  if (/^-?\d+\.\d+$/.test(v)) return 'decimal';
  return 'text';
}

/** Generalises the number runs of the first line into named groups and tests every line against the result. */
export function mockInferRegex(lines: string[]): InferredRegex {
  if (lines.length === 0) throw new Error('Add at least one example line.');
  const first = lines[0];
  const groups: InferredGroup[] = [];
  let pattern = '';
  let last = 0;
  for (const m of first.matchAll(/\d+(?:\.\d+)?/g)) {
    pattern += escape(first.slice(last, m.index));
    const name = `n${groups.length + 1}`;
    pattern += `(?P<${name}>${m[0].includes('.') ? '\\d+\\.\\d+' : '\\d+'})`;
    groups.push({ name, kind: kindOf(m[0]), examples: [] });
    last = m.index + m[0].length;
  }
  pattern += escape(first.slice(last));
  const re = toJsRegex(pattern);
  const found = lines.map((l) => re.exec(l));
  for (const g of groups) g.examples = found.flatMap((m) => (m?.groups?.[g.name] ? [m.groups[g.name]] : [])).slice(0, 3);
  return { pattern, groups, matches: found.map((m) => m !== null), explanation: mockExplainRegex(pattern) };
}
