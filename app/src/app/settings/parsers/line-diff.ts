/** One line of a two-text comparison, for display. */
export interface DiffLine {
  kind: 'same' | 'del' | 'add';
  text: string;
  /** 1-based line in the old text (null for an added line). */
  left: number | null;
  /** 1-based line in the new text (null for a removed line). */
  right: number | null;
}

/** Above this many line pairs the texts are shown as all removed, then all added (parser files are small). */
const MAX_CELLS = 400_000;

/**
 * A purely visual line diff of two small texts (longest common subsequence): what the import confirmation shows
 * between the parser in the library and the file being imported. It decides nothing.
 */
export function lineDiff(before: string, after: string): DiffLine[] {
  const a = before.replace(/\r\n/g, '\n').replace(/\n$/, '').split('\n');
  const b = after.replace(/\r\n/g, '\n').replace(/\n$/, '').split('\n');
  const n = a.length;
  const m = b.length;
  if (n * m > MAX_CELLS) {
    return [...a.map((text, i): DiffLine => ({ kind: 'del', text, left: i + 1, right: null })), ...b.map((text, j): DiffLine => ({ kind: 'add', text, left: null, right: j + 1 }))];
  }
  // lcs[i][j]: length of the common subsequence of a[i..] and b[j..].
  const lcs: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  for (let i = n - 1; i >= 0; i--) {
    const row = lcs[i]!;
    const next = lcs[i + 1]!;
    for (let j = m - 1; j >= 0; j--) row[j] = a[i] === b[j] ? next[j + 1]! + 1 : Math.max(next[j]!, row[j + 1]!);
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && a[i] === b[j]) {
      out.push({ kind: 'same', text: a[i]!, left: i + 1, right: j + 1 });
      i++;
      j++;
    } else if (i < n && (j >= m || lcs[i + 1]![j]! >= lcs[i]![j + 1]!)) {
      out.push({ kind: 'del', text: a[i]!, left: i + 1, right: null });
      i++;
    } else {
      out.push({ kind: 'add', text: b[j]!, left: null, right: j + 1 });
      j++;
    }
  }
  return out;
}
