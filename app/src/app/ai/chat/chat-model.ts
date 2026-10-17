/** Pure view model of a chat: turns built from saved records and from the streamed events of a running turn. */
import { ApprovalDecision, ChatContext, ChatEvent, Citation, Proposal, TurnRecord, Usage } from '../ai.types';

export interface ToolChip {
  name: string;
  args: string;
  running: boolean;
  summary: string | null;
  error: boolean;
}

export type ApprovalState = 'pending' | 'sending' | 'allowed' | 'edited' | 'allowedForChat' | 'denied';

export interface ApprovalView {
  requestId: number;
  kind: 'search' | 'fetch';
  text: string;
  domain: string | null;
  provider: string;
  editable: boolean;
  state: ApprovalState;
}

export interface ProposalView {
  proposal: Proposal;
  state: 'open' | 'applied' | 'dismissed';
}

export interface TurnView {
  user: string;
  text: string;
  citations: Citation[];
  proposals: ProposalView[];
  tools: ToolChip[];
  approvals: ApprovalView[];
  suggestions: { query: string; openUrl: string }[];
  notes: string[];
  error: string | null;
  cancelled: boolean;
  running: boolean;
  usage: Usage | null;
}

export function startTurn(user: string): TurnView {
  return { user, text: '', citations: [], proposals: [], tools: [], approvals: [], suggestions: [], notes: [], error: null, cancelled: false, running: true, usage: null };
}

export function turnFromRecord(r: TurnRecord): TurnView {
  const citations = [...(r.citations ?? [])];
  for (const w of r.web ?? []) if (!citations.some((c) => c.kind === 'web' && c.url === w.url)) citations.push({ kind: 'web', title: w.title, url: w.url });
  return {
    ...startTurn(r.user),
    text: r.assistant,
    citations,
    proposals: (r.proposals ?? []).filter((p) => !!p && typeof p === 'object' && 'kind' in p).map((proposal) => ({ proposal, state: 'open' as const })),
    cancelled: r.cancelled,
    running: false,
    usage: r.usage ?? null,
  };
}

/** Applies one streamed event to the running turn. */
export function reduceTurn(t: TurnView, e: ChatEvent): TurnView {
  switch (e.type) {
    case 'delta':
      return { ...t, text: t.text + e.text };
    case 'toolStarted':
      // The model's text before a tool call is a preamble, replaced by what it writes after the tools.
      return { ...t, text: '', tools: [...t.tools, { name: e.name, args: e.args, running: true, summary: null, error: false }] };
    case 'toolFinished': {
      const i = t.tools.map((c) => c.running && c.name === e.name).lastIndexOf(true);
      const done: ToolChip = { name: e.name, args: i >= 0 ? t.tools[i].args : '', running: false, summary: e.summary, error: e.error };
      const tools = i >= 0 ? t.tools.map((c, j) => (j === i ? done : c)) : [...t.tools, done];
      return { ...t, tools };
    }
    case 'proposal':
      return { ...t, proposals: [...t.proposals, { proposal: e.proposal, state: 'open' }] };
    case 'approvalRequired':
      return {
        ...t,
        approvals: [...t.approvals, { requestId: e.requestId, kind: e.kind, text: e.text, domain: e.domain, provider: e.provider, editable: e.editable, state: 'pending' }],
      };
    case 'searchSuggestion':
      return { ...t, suggestions: [...t.suggestions, { query: e.query, openUrl: e.openUrl }] };
    case 'done':
      return {
        ...t,
        text: e.text,
        citations: e.citations,
        notes: e.notes,
        usage: e.usage,
        running: false,
        tools: t.tools.map((c) => ({ ...c, running: false })),
        approvals: t.approvals.map((a) => (a.state === 'pending' || a.state === 'sending' ? { ...a, state: 'denied' } : a)),
      };
    case 'error':
      return {
        ...t,
        error: e.cancelled ? null : e.message,
        cancelled: e.cancelled,
        running: false,
        tools: t.tools.map((c) => ({ ...c, running: false })),
        approvals: t.approvals.map((a) => (a.state === 'pending' || a.state === 'sending' ? { ...a, state: 'denied' } : a)),
      };
  }
}

/** The state an approval shows after the user's decision. */
export function decidedState(d: ApprovalDecision): ApprovalState {
  switch (d.kind) {
    case 'allow':
      return 'allowed';
    case 'allowEdited':
      return 'edited';
    case 'allowSearchesInChat':
      return 'allowedForChat';
    case 'deny':
      return 'denied';
  }
}

/** Which parts of what the user looks at go with a question. */
export interface ContextToggles {
  filter: boolean;
  range: boolean;
  selection: boolean;
  instance: boolean;
  view: boolean;
}

export const ALL_CONTEXT: ContextToggles = { filter: true, range: true, selection: true, instance: true, view: true };

/** What the UI could send, before the toggles. */
export interface ContextSources {
  ql: string | null;
  range: { from: number; to: number } | null;
  selectedIds: number[];
  instance: string | null;
  view: string | null;
}

export function buildContext(src: ContextSources, on: ContextToggles): ChatContext {
  return {
    query: on.filter && src.ql?.trim() ? src.ql.trim() : null,
    timeRange: on.range && src.range ? [src.range.from, src.range.to] : null,
    selectedIds: on.selection ? src.selectedIds.slice(0, 20) : [],
    focusedInstance: on.instance ? src.instance : null,
    openView: on.view ? src.view : null,
  };
}

/** One line per part of the context, for the chip that shows what is sent. */
export function describeContext(src: ContextSources, fmt: (ms: number) => string): { key: keyof ContextToggles; label: string; available: boolean }[] {
  const n = src.selectedIds.length;
  return [
    { key: 'filter', label: src.ql?.trim() ? `Filter: ${src.ql.trim()}` : 'Filter: none', available: !!src.ql?.trim() },
    { key: 'range', label: src.range ? `Time: ${fmt(src.range.from)} – ${fmt(src.range.to)}` : 'Time: whole workspace', available: !!src.range },
    { key: 'selection', label: n ? `Selected: ${n} event${n === 1 ? '' : 's'} (#${src.selectedIds.slice(0, 3).join(', #')}${n > 3 ? ', …' : ''})` : 'Selected: none', available: n > 0 },
    { key: 'instance', label: src.instance ? `Instance: ${src.instance}` : 'Instance: all', available: !!src.instance },
    { key: 'view', label: src.view ? `View: ${src.view}` : 'View: none', available: !!src.view },
  ];
}

/** Human summary of a tool call for its chip ("run_query" → "Query"). */
export function toolLabel(name: string): string {
  const known: Record<string, string> = {
    run_query: 'Query',
    get_events: 'Read events',
    neighbours: 'Surrounding lines',
    field_stats: 'Field statistics',
    check_query: 'Check query',
    overview: 'Health check',
    exception_groups: 'Exceptions',
    patterns: 'Patterns',
    moment: 'Moment',
    anomalies: 'Anomalies',
    web_search: 'Web search',
    fetch_page: 'Read web page',
    propose_regex: 'Propose regex',
    propose_query: 'Propose query',
    propose_profile: 'Propose profile',
    propose_pipeline: 'Propose pipeline',
    propose_markers: 'Propose markers',
    propose_extractors: 'Propose extractors',
  };
  return known[name] ?? name.replace(/_/g, ' ');
}
