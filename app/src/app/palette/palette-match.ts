import { ImtCommand } from '../extensions/tokens';

/** A command as the palette lists it: its title, whether its category is shown beside it, and the matched characters. */
export interface PaletteEntry {
  command: ImtCommand;
  title: string;
  /** False when the title already names the category ("AI: Open the assistant chat" in AI): it is shown once. */
  showCategory: boolean;
  /** Character indexes into `title` that the query matched (for highlighting). */
  marks: ReadonlySet<number>;
}

/** A piece of a title: matched (`mark`) or not. */
export interface TitlePart {
  text: string;
  mark: boolean;
}

const WORD_START = /[\s:._\-/(]/;

function isWordStart(text: string, i: number): boolean {
  return i === 0 || WORD_START.test(text[i - 1] ?? '');
}

/**
 * Where one query word matches `text`: as the start of a word (best), inside a word (three letters or more), or as
 * the initials of consecutive words ("sa" → "Show anomalies"). Null when it does not match in one of these ways: a
 * scattered subsequence ("anom" in "Full screen does: hide tool windows…") is not a match.
 */
export function matchWord(token: string, text: string): { score: number; indices: number[] } | null {
  const t = text.toLowerCase();
  const q = token.toLowerCase();
  if (!q) return { score: 0, indices: [] };
  let at = t.indexOf(q);
  let inside: number | null = null;
  while (at >= 0) {
    if (isWordStart(t, at)) return { score: 10 + q.length * 2 - at / 100, indices: range(at, q.length) };
    inside ??= at;
    at = t.indexOf(q, at + 1);
  }
  if (inside !== null && q.length >= 3) return { score: 4 + q.length - inside / 100, indices: range(inside, q.length) };
  // Initials: each letter starts a word, in order.
  const starts = [...t].map((_, i) => i).filter((i) => isWordStart(t, i) && /\p{L}|\p{N}/u.test(t[i] ?? ''));
  const indices: number[] = [];
  let from = 0;
  for (const ch of q) {
    const k = starts.findIndex((s, j) => j >= from && t[s] === ch);
    if (k < 0) return null;
    indices.push(starts[k]!);
    from = k + 1;
  }
  return q.length >= 2 ? { score: 3 + q.length, indices } : null;
}

function range(start: number, length: number): number[] {
  return Array.from({ length }, (_, i) => start + i);
}

/** Whether the palette shows the command's category beside its title: not when the title starts with it already. */
export function showsCategory(c: Pick<ImtCommand, 'title' | 'category'>): boolean {
  const category = c.category?.trim().toLowerCase();
  return !!category && !c.title.toLowerCase().startsWith(`${category}:`);
}

/** The palette entry of a command, with no matched characters. */
export function plainEntry(command: ImtCommand): PaletteEntry {
  return { command, title: command.title, showCategory: showsCategory(command), marks: new Set<number>() };
}

/**
 * Ranks commands for the query: every word of the query must match the title (or the full label with its category) by {@link matchWord};
 * better and earlier matches first, then shorter titles. An empty query lists `recent` commands first, then
 * `common`, then the rest in registration order.
 */
export function rankCommands(commands: readonly ImtCommand[], query: string, recent: readonly string[] = [], common: readonly string[] = []): PaletteEntry[] {
  const tokens = query.trim().split(/\s+/).filter(Boolean);
  if (!tokens.length) {
    const order = (id: string) => {
      const r = recent.indexOf(id);
      if (r >= 0) return r;
      const c = common.indexOf(id);
      return c >= 0 ? recent.length + c : Number.POSITIVE_INFINITY;
    };
    return commands
      .map((command, index) => ({ command, index, rank: order(command.id) }))
      .sort((a, b) => a.rank - b.rank || a.index - b.index)
      .map(({ command }) => plainEntry(command));
  }
  const scored: { entry: PaletteEntry; score: number; index: number }[] = [];
  commands.forEach((command, index) => {
    const title = command.title;
    let score = 0;
    const marks = new Set<number>();
    // The full label ("Theme: console") also counts, so a query typed the way the label reads still matches.
    const label = command.category ? `${command.category}: ${command.title}` : command.title;
    for (const token of tokens) {
      const inTitle = matchWord(token, title);
      const inLabel = matchWord(token, label);
      if (!inTitle && !inLabel) return;
      if (inTitle && (!inLabel || inTitle.score >= inLabel.score / 2)) {
        score += inTitle.score;
        inTitle.indices.forEach((i) => marks.add(i));
      } else if (inLabel) {
        // A match in the category counts for less: "view" should not rank every View command above "Overview".
        score += inLabel.score / 2;
      }
    }
    scored.push({ entry: { command, title, showCategory: showsCategory(command), marks }, score: score - title.length / 1000, index });
  });
  return scored.sort((a, b) => b.score - a.score || a.index - b.index).map((s) => s.entry);
}

/** Splits a title into matched and unmatched runs for rendering. */
export function titleParts(title: string, marks: ReadonlySet<number>): TitlePart[] {
  const parts: TitlePart[] = [];
  for (let i = 0; i < title.length; i++) {
    const mark = marks.has(i);
    const last = parts.at(-1);
    if (last && last.mark === mark) last.text += title[i];
    else parts.push({ text: title[i]!, mark });
  }
  return parts;
}
