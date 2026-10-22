import { KindHits, LEVELS, Query, SearchAllReply, SearchAllRequest, SnippetPart } from '../../core/api.types';
import type { ImtBackend } from '../../core/backend';
import { CmdError } from '../../core/cmd-error';
import type { SearchEverythingBackend } from '../search-everything-backend';

const DEFAULT_PER_KIND = 8;
const MAX_PER_KIND = 50;
const MAX_TEXT_CHARS = 256;
const SNIPPET_CHARS = 200;

const words = (text: string) => text.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean);

/** A snippet with every word marked (simplified: the engine's is the reference). */
export function mockSnippet(text: string, needles: string[]): SnippetPart[] {
  const shown = text.replace(/\s+/g, ' ').trim().slice(0, SNIPPET_CHARS);
  const lower = shown.toLowerCase();
  const marks: [number, number][] = [];
  for (const n of needles) for (let at = lower.indexOf(n); at >= 0; at = lower.indexOf(n, at + n.length)) marks.push([at, at + n.length]);
  marks.sort((a, b) => a[0] - b[0]);
  const parts: SnippetPart[] = [];
  let at = 0;
  for (const [s, e] of marks) {
    if (s < at) continue;
    if (s > at) parts.push({ text: shown.slice(at, s), mark: false });
    parts.push({ text: shown.slice(s, e), mark: true });
    at = e;
  }
  if (at < shown.length) parts.push({ text: shown.slice(at), mark: false });
  return parts;
}

function hits<T>(all: T[], limit: number): KindHits<T> {
  return { total: all.length, items: all.slice(0, limit), outcome: { kind: 'complete' } };
}

/**
 * Search everything over the mock workspace (browser development and UI tests only; the engine is the reference): events
 * from the mock's word search, sources, field names and marker kinds from its summary; no notes or knowledge files.
 */
export class MockSearchEverythingBackend implements SearchEverythingBackend {
  constructor(private readonly workspace: ImtBackend) {}

  async searchEverything(request: SearchAllRequest): Promise<SearchAllReply | null> {
    const text = request.text.trim();
    if (text.length > MAX_TEXT_CHARS) throw new CmdError({ code: 'invalid', message: `search for at most ${MAX_TEXT_CHARS} characters`, retryable: false });
    const needles = words(text);
    if (!needles.length) return null;
    const limit = Math.min(request.limit || DEFAULT_PER_KIND, MAX_PER_KIND);
    const holds = (...texts: string[]) => {
      const have = texts.join(' ').toLowerCase();
      return needles.every((n) => have.includes(n));
    };
    const query: Query = { levels: [], text, textMode: 'words', regex: false, caseSensitive: false, filters: [], groupBy: [], groups: [], markers: [] };
    const [summary, page] = await Promise.all([this.workspace.workspaceSummary(), this.workspace.queryPage({ query, offset: 0, limit, descending: false })]);
    return {
      events: {
        total: page.total,
        items: page.rows.map((r) => ({ id: r.id, ts: r.ts, level: LEVELS.includes(r.level) ? r.level : 'Information', source: r.sourceName, snippet: mockSnippet(r.message, needles) })),
        outcome: { kind: 'complete' },
      },
      sources: hits(
        summary.sources.filter((s) => holds(s.path, s.name)).map((s) => ({ id: s.id, name: s.name, path: s.path, records: s.records, snippet: mockSnippet(s.path, needles) })),
        limit,
      ),
      fields: hits(
        summary.fields.filter((f) => holds(f.name)).map((f) => ({ field: f.name, value: null, count: f.count, snippet: mockSnippet(f.name, needles) })),
        limit,
      ),
      markers: hits(
        summary.markerKinds.filter((m) => holds(m.kind, m.label)).map((m) => ({ kind: m.kind, snippet: mockSnippet(m.kind, needles) })),
        limit,
      ),
      notes: hits([], limit),
      knowledge: hits([], limit),
    };
  }
}
