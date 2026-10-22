import { DocHit, DocScope, KindHits, KindOutcome, SearchAllReply, SnippetPart } from '../core/api.types';

/** Where choosing a result goes. */
export type SearchTarget =
  | { kind: 'event'; id: number }
  /** Every matching event: the grid's smart search with the same text. */
  | { kind: 'allEvents'; text: string }
  | { kind: 'source'; id: number }
  | { kind: 'field'; field: string; value: string | null }
  | { kind: 'marker'; marker: string }
  | { kind: 'note'; id: string; event: number | null }
  | { kind: 'knowledge'; scope: DocScope; id: string };

export type ResultKind = keyof SearchAllReply;

/** One choosable line of the palette. */
export interface ResultRow {
  /** Unique within the results (for `track` and option ids). */
  key: string;
  snippet: SnippetPart[];
  /** Secondary text (source, level, count, scope). */
  detail: string;
  /** The event's time, formatted by the component. */
  ts: number | null;
  target: SearchTarget;
}

/** The results of one kind, as listed. */
export interface ResultGroup {
  kind: ResultKind;
  title: string;
  total: number;
  /** Why the kind is incomplete (timed out, failed); null when it was searched completely. */
  notice: string | null;
  rows: ResultRow[];
}

/** The kinds in the order they are listed. */
export const KIND_ORDER: readonly ResultKind[] = ['events', 'sources', 'fields', 'markers', 'notes', 'knowledge'];

function kindTitle(kind: ResultKind): string {
  switch (kind) {
    case 'events':
      return $localize`:@@search.kind.events:Events`;
    case 'sources':
      return $localize`:@@search.kind.sources:Sources`;
    case 'fields':
      return $localize`:@@search.kind.fields:Fields`;
    case 'markers':
      return $localize`:@@search.kind.markers:Markers`;
    case 'notes':
      return $localize`:@@search.kind.notes:Notes`;
    case 'knowledge':
      return $localize`:@@search.kind.knowledge:Knowledge`;
  }
}

/** What the palette says about a kind that was not searched completely. */
export function outcomeNotice(outcome: KindOutcome): string | null {
  switch (outcome.kind) {
    case 'complete':
      return null;
    case 'timedOut':
      return $localize`:@@search.outcome.timedOut:Search time ran out; there may be more matches.`;
    case 'failed':
      return $localize`:@@search.outcome.failed:Could not search: ${outcome.message}:message:`;
  }
}

const plain = (text: string): SnippetPart[] => [{ text, mark: false }];

function docRows(kind: 'notes' | 'knowledge', hits: KindHits<DocHit>): ResultRow[] {
  return hits.items.map((d) => ({
    key: `${kind}:${d.scope}:${d.id}`,
    snippet: d.snippet.length ? d.snippet : plain(d.title),
    detail: kind === 'notes' ? d.title : `${d.title} · ${d.scope === 'global' ? $localize`:@@search.scope.global:Global` : $localize`:@@search.scope.project:Project`}`,
    ts: null,
    target: kind === 'notes' ? { kind: 'note', id: d.id, event: d.event } : { kind: 'knowledge', scope: d.scope, id: d.id },
  }));
}

/** The rows of one kind (with "Show all in the grid" after the events when there are more than shown). */
function rowsOf(kind: ResultKind, reply: SearchAllReply, text: string): ResultRow[] {
  switch (kind) {
    case 'events': {
      const rows: ResultRow[] = reply.events.items.map((e) => ({ key: `event:${e.id}`, snippet: e.snippet, detail: `${e.level} · ${e.source}`, ts: e.ts, target: { kind: 'event', id: e.id } }));
      if (reply.events.total > reply.events.items.length)
        rows.push({
          key: 'events:all',
          snippet: plain($localize`:@@search.events.all:Show all ${reply.events.total}:count: matching events in the grid`),
          detail: '',
          ts: null,
          target: { kind: 'allEvents', text },
        });
      return rows;
    }
    case 'sources':
      return reply.sources.items.map((s) => ({ key: `source:${s.id}`, snippet: s.snippet, detail: $localize`:@@search.source.records:${s.records}:count: events`, ts: null, target: { kind: 'source', id: s.id } }));
    case 'fields':
      return reply.fields.items.map((f) => ({
        key: `field:${f.field}:${f.value ?? ''}`,
        snippet: f.value === null ? f.snippet : [{ text: `${f.field} = `, mark: false }, ...f.snippet],
        detail: $localize`:@@search.field.count:${f.count}:count: events`,
        ts: null,
        target: { kind: 'field', field: f.field, value: f.value },
      }));
    case 'markers':
      return reply.markers.items.map((m) => ({ key: `marker:${m.kind}`, snippet: m.snippet, detail: '', ts: null, target: { kind: 'marker', marker: m.kind } }));
    case 'notes':
    case 'knowledge':
      return docRows(kind, reply[kind]);
  }
}

/** The palette's groups: kinds with results or with something to say, in {@link KIND_ORDER}. */
export function resultGroups(reply: SearchAllReply | null, text: string): ResultGroup[] {
  if (!reply) return [];
  return KIND_ORDER.map((kind) => ({ kind, title: kindTitle(kind), total: reply[kind].total, notice: outcomeNotice(reply[kind].outcome), rows: rowsOf(kind, reply, text) })).filter(
    (g) => g.rows.length > 0 || g.notice !== null,
  );
}

/** Every row of the groups, in order (what the keyboard moves through). */
export function flatRows(groups: ResultGroup[]): ResultRow[] {
  return groups.flatMap((g) => g.rows);
}
