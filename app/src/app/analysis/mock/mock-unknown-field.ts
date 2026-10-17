import { QlError } from '../../core/api.types';
import { lex } from '../../core/mock/mock-ql';
import { editDistance } from '../../palette/did-you-mean';

/** Names the mock query language always understands (pseudo-fields and their bare forms). */
const BUILTIN = ['@level', '@message', '@exception', '@template', '@source', '@line', 'level', 'message', 'exception', 'source', 'template', 'line'];

/** UTF-8 byte offset of the UTF-16 index `index`. */
function bytes(text: string, index: number): number {
  return new TextEncoder().encode(text.slice(0, index)).length;
}

/**
 * Browser stand-in for the engine's unknown-field check in `check_query`: the first name compared with an
 * operator that no event carries, with up to three close known names (case-insensitive edit distance).
 * Null when every compared name is known, or when there are no fields to compare with (empty workspace).
 */
export function mockUnknownField(text: string, fields: readonly string[]): QlError | null {
  if (!fields.length) return null;
  const known = [...BUILTIN, ...fields];
  const lower = new Set(known.map((k) => k.toLowerCase()));
  let toks: ReturnType<typeof lex>;
  try {
    toks = lex(text);
  } catch {
    return null;
  }
  for (let i = 0; i + 1 < toks.length; i++) {
    const t = toks[i];
    const next = toks[i + 1];
    const compared = next.kind === 'op' || (next.kind === 'word' && ['contains', 'in'].includes(next.text.toLowerCase()));
    if (t.kind !== 'word' || !compared || lower.has(t.text.toLowerCase())) continue;
    const limit = Math.max(2, Math.floor(t.text.length / 3));
    const suggestions = known
      .map((name) => ({ name, d: editDistance(t.text, name) }))
      .filter((x) => x.d <= limit)
      .sort((a, b) => a.d - b.d)
      .map((x) => x.name)
      .filter((n, idx, all) => all.findIndex((m) => m.toLowerCase() === n.toLowerCase()) === idx)
      .slice(0, 3);
    const offset = bytes(text, t.offset);
    return { message: `unknown field "${t.text}"`, offset, len: bytes(text, t.offset + t.len) - offset, suggestions };
  }
  return null;
}
