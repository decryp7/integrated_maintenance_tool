/**
 * Scores how well `query` matches `text` as an in-order subsequence (case-insensitive).
 * Returns null when it does not match. Higher is better: consecutive characters and matches
 * at word starts score more, and shorter texts win ties.
 */
export function fuzzyScore(query: string, text: string): number | null {
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  const t = text.toLowerCase();
  let score = 0;
  let ti = 0;
  let prev = -2;
  for (const ch of q) {
    const found = t.indexOf(ch, ti);
    if (found < 0) return null;
    score += 1;
    if (found === prev + 1) score += 3;
    if (found === 0 || /[\s:._\-/]/.test(t[found - 1] ?? '')) score += 2;
    prev = found;
    ti = found + 1;
  }
  return score - t.length / 1000;
}

/** Filters and ranks items by fuzzy score on `label`; keeps the original order for an empty query. */
export function fuzzyFilter<T>(items: readonly T[], query: string, label: (item: T) => string): T[] {
  if (!query.trim()) return [...items];
  return items
    .map((item, index) => ({ item, index, score: fuzzyScore(query, label(item)) }))
    .filter((x): x is { item: T; index: number; score: number } => x.score !== null)
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map((x) => x.item);
}
