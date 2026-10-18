/** A line of a diff: kept in both, only in the old text (removed) or only in the new one (added). */
export interface DiffLine {
  kind: 'same' | 'removed' | 'added';
  text: string;
}

export interface LineDiff {
  lines: DiffLine[];
  added: number;
  removed: number;
  /** The changed middle was too large to align line by line: it is shown as removed then added. */
  coarse: boolean;
}

/** Cells of the LCS table above which the middle is not aligned (keeps the UI responsive on big profiles). */
export const MAX_DIFF_CELLS = 2_000_000;

/**
 * Line diff of `before` → `after` (longest common subsequence after trimming the common head and tail).
 * Purely visual: profile comparison for the history list.
 */
export function lineDiff(before: string, after: string, maxCells = MAX_DIFF_CELLS): LineDiff {
  const a = before.split(/\r?\n/);
  const b = after.split(/\r?\n/);
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head++;
  let tail = 0;
  while (tail < a.length - head && tail < b.length - head && a[a.length - 1 - tail] === b[b.length - 1 - tail]) tail++;
  const midA = a.slice(head, a.length - tail);
  const midB = b.slice(head, b.length - tail);
  const out: DiffLine[] = a.slice(0, head).map((text) => ({ kind: 'same', text }));
  let coarse = false;
  if (midA.length * midB.length > maxCells) {
    coarse = true;
    out.push(...midA.map((text): DiffLine => ({ kind: 'removed', text })), ...midB.map((text): DiffLine => ({ kind: 'added', text })));
  } else {
    out.push(...lcs(midA, midB));
  }
  out.push(...a.slice(a.length - tail).map((text): DiffLine => ({ kind: 'same', text })));
  return { lines: out, added: out.filter((l) => l.kind === 'added').length, removed: out.filter((l) => l.kind === 'removed').length, coarse };
}

function lcs(a: string[], b: string[]): DiffLine[] {
  const n = a.length;
  const m = b.length;
  // len[i][j] = LCS length of a[i..] and b[j..].
  const len = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
  const cell = (r: number, c: number) => len[r]?.[c] ?? 0;
  for (let i = n - 1; i >= 0; i--) {
    const row = len[i];
    if (!row) continue;
    for (let j = m - 1; j >= 0; j--) row[j] = a[i] === b[j] ? cell(i + 1, j + 1) + 1 : Math.max(cell(i + 1, j), cell(i, j + 1));
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: 'same', text: a[i] ?? '' });
      i++;
      j++;
    } else if (cell(i + 1, j) >= cell(i, j + 1)) {
      out.push({ kind: 'removed', text: a[i++] ?? '' });
    } else {
      out.push({ kind: 'added', text: b[j++] ?? '' });
    }
  }
  while (i < n) out.push({ kind: 'removed', text: a[i++] ?? '' });
  while (j < m) out.push({ kind: 'added', text: b[j++] ?? '' });
  return out;
}

/** Turns the stamp of a saved version (`YYYYMMDD-HHMMSSmmm`, UTC) into epoch ms; null when it is not one. */
export function stampToMs(stamp: string): number | null {
  const m = /^(\d{4})(\d{2})(\d{2})-(\d{2})(\d{2})(\d{2})(\d{3})?/.exec(stamp);
  if (!m) return null;
  const [, y = '', mo = '', d = '', h = '', mi = '', s = '', ms] = m;
  const t = Date.UTC(+y, +mo - 1, +d, +h, +mi, +s, +(ms ?? 0));
  return Number.isNaN(t) ? null : t;
}
