import type { KnowledgeBackend } from '../../ai/knowledge/knowledge-backend';
import type { AnalysisBackend } from '../../core/analysis-backend';
import { DocHit, FindValuesReply, FindValuesRequest, NameFactReply, NameFeedback, NameRemovedReply, NameUndoReply, RemoveNameAlias, ValueMatch, DocScope, FieldHit, KindHits, LEVELS, Query, SearchAllReply, SearchAllRequest, SnippetPart } from '../../core/api.types';
import type { DebriefBackend } from '../../core/backend';
import type { NotesBackend } from '../../notes/notes-backend';
import { CmdError } from '../../core/cmd-error';
import type { SearchEverythingBackend } from '../search-everything-backend';

const DEFAULT_PER_KIND = 8;
const MAX_PER_KIND = 50;
const MAX_TEXT_CHARS = 256;
const SNIPPET_CHARS = 200;
const NOTE_TITLE_CHARS = 80;
/** Like the engine: fields with more distinct values are searched by name only. */
const MAX_DISTINCT_VALUES = 500;
const MAX_VALUE_FIELDS = 24;

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

/** A note's title: its first line, shortened. */
function noteTitle(text: string): string {
  const line = text.split('\n').map((l) => l.trim()).find((l) => l) ?? '';
  return line.length > NOTE_TITLE_CHARS ? `${line.slice(0, NOTE_TITLE_CHARS)}…` : line;
}

/** One entity of the mock name book: the names the mock logs hold for one thing, and the AI aliases suggested for it. */
interface MockEntity {
  id: string;
  display: string;
  kind: string;
  spellings: { name: string; count: number }[];
  evidence: number[];
  aliases: string[];
}

/** Invented entities, enough to show a match with its reasons and a tie ("builder" is two things). */
const MOCK_ENTITIES: readonly MockEntity[] = [
  {
    id: 'executable|graphic builder',
    display: 'Graphic Builder',
    kind: 'executable',
    spellings: [
      { name: 'graphicbuilder.exe', count: 9 },
      { name: 'CompanyA.Graphic.Builder.exe', count: 5 },
      { name: 'graphic-builder.exe', count: 2 },
    ],
    evidence: [3, 7, 12],
    aliases: ['image editor'],
  },
  {
    id: 'executable|report builder',
    display: 'Report Builder',
    kind: 'executable',
    spellings: [
      { name: 'CompanyA.Report.Builder.exe', count: 7 },
      { name: 'reportbuilder.exe', count: 3 },
    ],
    evidence: [5, 9, 14],
    aliases: [],
  },
];

const squashed = (t: string) => words(t).join('');

/** The mock's way of matching, best first (the engine's `How`, simplified). */
type MockVia = ValueMatch['via'];
const VIA_SCORE: Record<MockVia, number> = { userAlias: 1.3, exact: 1, segmented: 0.95, parts: 0.85, alias: 0.8, prefix: 0.7, fuzzy: 0.55 };

/** The other mock backends the search reads, resolved when a search needs them (so a missing one fails only its kind). */
export interface MockSearchSources {
  notes: () => NotesBackend;
  knowledge: () => KnowledgeBackend;
  analysis: () => AnalysisBackend;
}

/**
 * Search everything over the mock workspace (browser development and UI tests only; the engine is the reference): events
 * from the mock's word search, sources, field names and the values of fields with few values, marker kinds, exception
 * groups, notes and knowledge files.
 */
export class MockSearchEverythingBackend implements SearchEverythingBackend {
  constructor(
    private readonly workspace: DebriefBackend,
    private readonly sources: MockSearchSources,
  ) {}

  private async fieldHits(fields: { name: string; count: number; distinct: number; numeric: boolean }[], needles: string[], query: Query, holds: (...t: string[]) => boolean): Promise<FieldHit[]> {
    const hits: FieldHit[] = fields.filter((f) => holds(f.name)).map((f) => ({ field: f.name, value: null, count: f.count, snippet: mockSnippet(f.name, needles) }));
    const searched = fields.filter((f) => !f.numeric && f.distinct > 0 && f.distinct <= MAX_DISTINCT_VALUES).slice(0, MAX_VALUE_FIELDS);
    for (const f of searched) {
      const facet = await this.workspace.queryFacet({ query: { ...query, text: '' }, field: f.name, limit: MAX_DISTINCT_VALUES });
      for (const v of facet.values) if (holds(v.value)) hits.push({ field: f.name, value: v.value, count: v.count, snippet: mockSnippet(v.value, needles) });
    }
    return hits;
  }

  private async noteHits(needles: string[], holds: (...t: string[]) => boolean): Promise<DocHit[]> {
    const { notes } = await this.sources.notes().listNotes();
    return notes
      .filter((n) => holds(noteTitle(n.text), n.text))
      .map((n) => ({ id: n.id, scope: 'project' as DocScope, title: noteTitle(n.text), event: n.anchor.kind === 'event' ? n.anchor.eventId : null, snippet: mockSnippet(n.text, needles) }));
  }

  private async knowledgeHits(needles: string[], holds: (...t: string[]) => boolean): Promise<DocHit[]> {
    const backend = this.sources.knowledge();
    const list = await backend.listKnowledge();
    const hits: DocHit[] = [];
    for (const f of [...list.project, ...list.global]) {
      const { text } = await backend.getKnowledge(f.scope, f.id);
      if (holds(f.name, text)) hits.push({ id: f.id, scope: f.scope, title: f.name, event: null, snippet: mockSnippet(text, needles) });
    }
    return hits;
  }

  /** What the user told the mock: the words that mean an entity, the ones that do not, and the order to undo them in. */
  private readonly meant = new Map<string, string>();
  private readonly notMeant = new Set<string>();
  private readonly facts: { id: number; undo: () => void }[] = [];
  private readonly removedAliases = new Set<string>();
  private nextFact = 1;

  /** How the words match an entity of the mock name book, or null. */
  private matchEntity(e: MockEntity, text: string): { via: MockVia; why: string } | null {
    const q = squashed(text);
    if (this.notMeant.has(`${q}|${e.id}`)) return null;
    if (this.meant.get(q) === e.id) return { via: 'userAlias', why: 'you told Debrief this name' };
    if (q === squashed(e.display)) return { via: 'segmented', why: 'same name written differently' };
    if (e.aliases.some((a) => squashed(a) === q && !this.removedAliases.has(`${e.id}\t${a}`))) return { via: 'alias', why: 'AI alias (AI-generated)' };
    const typed = words(text);
    const own = words(e.display);
    if (typed.length && typed.every((w) => own.includes(w))) return { via: 'parts', why: 'your words are parts of the name' };
    if (typed.length && typed.every((w) => own.some((o) => o.startsWith(w)))) return { via: 'prefix', why: 'your words start the words of the name' };
    return null;
  }

  /**
   * Entities of the mock name book (with spellings, why and evidence; a tie asks a question), then source names whose
   * letters and digits hold the words' (separators and case ignored). The engine is the reference.
   */
  async findValues(request: FindValuesRequest): Promise<FindValuesReply> {
    const wanted = squashed(request.text);
    if (!wanted) return { values: [] };
    const limit = request.limit || 5;
    const matched = MOCK_ENTITIES.flatMap((e) => {
      const m = this.matchEntity(e, request.text);
      return m ? [{ e, ...m }] : [];
    }).sort((a, b) => VIA_SCORE[b.via] - VIA_SCORE[a.via]);
    const entities: ValueMatch[] = matched.map(({ e, via, why }) => ({
      field: '@entity',
      value: e.spellings[0]?.name ?? e.display,
      count: e.spellings.reduce((n, s) => n + s.count, 0),
      via,
      kind: e.kind,
      why,
      spellings: e.spellings,
      evidence: e.evidence,
      group: e.id,
    }));
    const [first, second] = matched;
    const tie = first && second && first.via === second.via;
    const summary = await this.workspace.workspaceSummary();
    const sources: ValueMatch[] = summary.sources
      .filter((s) => squashed(s.name).includes(wanted))
      .map((s) => ({ field: '@source', value: s.name, count: s.records, via: squashed(s.name) === wanted ? ('exact' as const) : ('parts' as const), why: 'the source name holds your words' }));
    const values = [...entities, ...sources].slice(0, limit);
    return tie ? { values, clarify: { question: `Which one do you mean: ${first.e.display} or ${second.e.display}?`, options: [first.e.id, second.e.id] } } : { values };
  }

  private known(group: string): MockEntity {
    const e = MOCK_ENTITIES.find((x) => x.id === group);
    if (!e) throw new CmdError({ code: 'invalid', message: 'that name is no longer in the logs; search again', retryable: false });
    return e;
  }

  private phrase(text: string): string {
    const t = text.trim();
    if (!t || t.length > 128) throw new CmdError({ code: 'invalid', message: 'the words must be 1 to 128 characters', retryable: false });
    return squashed(t);
  }

  async acceptName(feedback: NameFeedback): Promise<NameFactReply> {
    const q = this.phrase(feedback.text);
    const e = this.known(feedback.group);
    const before = this.meant.get(q);
    this.meant.set(q, e.id);
    const id = this.nextFact++;
    this.facts.push({ id, undo: () => (before === undefined ? this.meant.delete(q) : this.meant.set(q, before)) });
    return { fact: id };
  }

  async rejectName(feedback: NameFeedback): Promise<NameFactReply> {
    const key = `${this.phrase(feedback.text)}|${this.known(feedback.group).id}`;
    const had = this.notMeant.has(key);
    this.notMeant.add(key);
    const id = this.nextFact++;
    this.facts.push({ id, undo: () => void (had || this.notMeant.delete(key)) });
    return { fact: id };
  }

  async undoName(): Promise<NameUndoReply> {
    const f = this.facts.pop();
    f?.undo();
    return { undone: f ? { id: f.id, kind: {}, global: false } : null };
  }

  async removeNameAlias(request: RemoveNameAlias): Promise<NameRemovedReply> {
    const e = this.known(request.group);
    const alias = request.alias.trim();
    const key = `${e.id}\t${alias}`;
    if (!e.aliases.includes(alias) || this.removedAliases.has(key)) return { removed: false };
    this.removedAliases.add(key);
    return { removed: true };
  }

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
    const [summary, page, groups, notes, knowledge] = await Promise.all([
      this.workspace.workspaceSummary(),
      this.workspace.queryPage({ query, offset: 0, limit, descending: false }),
      this.sources.analysis().queryExceptions({ ...query, text: '' }),
      this.noteHits(needles, holds),
      this.knowledgeHits(needles, holds),
    ]);
    const fields = await this.fieldHits(summary.fields, needles, query, holds);
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
      fields: hits(fields, limit),
      markers: hits(
        summary.markerKinds.filter((m) => holds(m.kind, m.label)).map((m) => ({ kind: m.kind, snippet: mockSnippet(m.kind, needles) })),
        limit,
      ),
      exceptions: hits(
        groups.result
          .filter((g) => holds(g.typeName, g.message, g.topFrame ?? ''))
          .map((g) => ({ fingerprint: g.fingerprint, typeName: g.typeName, count: g.count, sample: g.sampleIds[0] ?? null, snippet: mockSnippet(`${g.typeName}: ${g.message}`, needles) })),
        limit,
      ),
      notes: hits(notes, limit),
      knowledge: hits(knowledge, limit),
    };
  }
}
