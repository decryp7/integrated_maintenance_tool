/**
 * "Did you mean": typo-tolerant suggestions for the command palette when nothing matches. Pure and
 * presentation-only: commands are a UI concept (field-name suggestions come from the engine).
 */

/** Optimal-string-alignment distance (insertions, deletions, substitutions, adjacent swaps), case-insensitive. */
export function editDistance(a: string, b: string): number {
  const s = a.toLowerCase();
  const t = b.toLowerCase();
  if (s === t) return 0;
  if (!s.length) return t.length;
  if (!t.length) return s.length;
  let prev2: number[] = [];
  let prev = Array.from({ length: t.length + 1 }, (_, j) => j);
  for (let i = 1; i <= s.length; i++) {
    const cur = [i];
    for (let j = 1; j <= t.length; j++) {
      const cost = s[i - 1] === t[j - 1] ? 0 : 1;
      let v = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + cost);
      if (i > 1 && j > 1 && s[i - 1] === t[j - 2] && s[i - 2] === t[j - 1]) v = Math.min(v, prev2[j - 2] + 1);
      cur.push(v);
    }
    prev2 = prev;
    prev = cur;
  }
  return prev[t.length];
}

/** Typos tolerated in a word of this length: one per three characters, at least one. */
export function tolerance(word: string): number {
  return Math.max(1, Math.floor(word.length / 3));
}

const WORDS = /[\p{L}\p{N}]+/gu;

function words(text: string): string[] {
  return text.toLowerCase().match(WORDS) ?? [];
}

/**
 * How far `query` is from `label`: every query word is matched with its closest label word (a prefix of
 * a label word counts as exact) and the distances are summed. Null when some word is too far off.
 */
export function suggestionScore(query: string, label: string): number | null {
  const q = words(query);
  const l = words(label);
  if (!q.length || !l.length) return null;
  let total = 0;
  for (const w of q) {
    let best = Infinity;
    for (const x of l) {
      const d = x.startsWith(w) ? 0 : Math.min(editDistance(w, x), editDistance(w, x.slice(0, w.length)));
      best = Math.min(best, d);
    }
    if (best > tolerance(w)) return null;
    total += best;
  }
  return total;
}

/** Up to `limit` items whose label is closest to `query`, best first (ties keep the original order). */
export function didYouMean<T>(items: readonly T[], query: string, label: (item: T) => string, limit = 3): T[] {
  return items
    .map((item, index) => ({ item, index, score: suggestionScore(query, label(item)) }))
    .filter((x): x is { item: T; index: number; score: number } => x.score !== null)
    .sort((a, b) => a.score - b.score || a.index - b.index)
    .slice(0, limit)
    .map((x) => x.item);
}
