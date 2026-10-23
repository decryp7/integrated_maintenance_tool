/** The ellipsis character that marks removed text. */
export const ELLIPSIS = '…';

/** Longest file name (in characters) kept whole when a path is shortened; a longer name keeps its last characters. */
export const DEFAULT_KEEP_TAIL = 20;

const isSeparator = (c: string | undefined): boolean => c === '/' || c === '\\';

function lastSeparatorIndex(chars: readonly string[]): number {
  for (let i = chars.length - 1; i >= 0; i--) if (isSeparator(chars[i])) return i;
  return -1;
}

/** A text cut into the part that may be shortened and the part that always stays visible. */
export interface SplitText {
  head: string;
  tail: string;
}

/**
 * Splits `text` for a middle ellipsis done by layout (CSS): `tail` is the last separator and file name (or, with no
 * separator or a name longer than `keepTail`, the last `keepTail` characters, so a long extension or id stays visible)
 * and `head` is the rest, which the layout shortens with an ellipsis. Counts code points, never splitting a surrogate pair.
 */
export function splitForEllipsis(text: string, keepTail = DEFAULT_KEEP_TAIL): SplitText {
  const chars = Array.from(text);
  const keep = Math.max(0, Math.floor(keepTail));
  let start = chars.length - keep;
  const lastSeparator = lastSeparatorIndex(chars);
  // Keep the separator before the file name when the whole name fits: "…\Builder.Service.log".
  if (lastSeparator >= 0 && chars.length - lastSeparator <= keep) start = lastSeparator;
  if (start <= 0) return { head: '', tail: text };
  return { head: chars.slice(0, start).join(''), tail: chars.slice(start).join('') };
}

/**
 * Shortens `text` to at most `max` characters (code points) by replacing the middle with an ellipsis. A path keeps its
 * start and its file name ("C:\ProgramData\…\Builder.Service.log"); other text keeps its start and its end. `max`
 * below 1 gives an empty string and 1 gives only the ellipsis. Text that fits is returned unchanged.
 */
export function middleEllipsis(text: string, max: number): string {
  const chars = Array.from(text);
  const limit = Math.floor(max);
  if (!(limit >= 1)) return '';
  if (chars.length <= limit) return text;
  if (limit === 1) return ELLIPSIS;
  const budget = limit - 1;
  const lastSeparator = lastSeparatorIndex(chars);
  const nameLength = chars.length - lastSeparator; // with its separator
  // Path: keep the separator and file name when at least two characters of the start fit as well.
  if (lastSeparator > 0 && nameLength <= budget - 2) {
    const headBudget = budget - nameLength;
    let head = chars.slice(0, headBudget);
    // Prefer ending the start on a separator ("C:\ProgramData\") when that keeps at least half of it.
    const cut = lastSeparatorIndex(head);
    if (cut + 1 >= Math.ceil(headBudget / 2)) head = head.slice(0, cut + 1);
    return head.join('') + ELLIPSIS + chars.slice(lastSeparator).join('');
  }
  const tail = Math.floor(budget / 2);
  const head = budget - tail;
  return chars.slice(0, head).join('') + ELLIPSIS + chars.slice(chars.length - tail).join('');
}
