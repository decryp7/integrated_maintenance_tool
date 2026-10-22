import type { ParserEntryDto } from '../../core/api.types';

/** What the Settings search finds in the Parsers section: the section itself and the parsers whose text matches. */
export interface ParserSearchHit {
  /** The words match the section (its title or what it is about). */
  section: boolean;
  /** Parsers of the library whose name, id, category or description hold every word. */
  parsers: ParserEntryDto[];
}

/** Words the Parsers section answers to, besides its title. */
const SECTION_WORDS = ['parsers', 'parser', 'log formats', 'format', 'parse with', 'read files'];

/**
 * The Settings search over the Parsers section (the same rule as for engine settings: every word must appear), or
 * null when nothing there matches. Visual matching only: the library itself is the engine's.
 */
export function searchParsers(parsers: readonly ParserEntryDto[], query: string, title: string): ParserSearchHit | null {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return null;
  const all = (haystack: string) => words.every((w) => haystack.toLowerCase().includes(w));
  const section = all(`${title} ${SECTION_WORDS.join(' ')}`);
  const hits = parsers.filter((p) => all(`${p.name} ${p.id} ${p.category} ${p.description}`));
  return section || hits.length ? { section, parsers: hits } : null;
}
