/** Pure view model of a chat: turns built from saved records and from the streamed events of a running turn. */
import { plural } from '../../i18n/locale';
import { AgentApprovalRequest, AgentDecision, HandOff, PauseReason, Segment } from '../../browser/browser.types';
import { ChatContext, ChatEvent, Citation, Proposal, TurnRecord, Usage } from '../ai.types';

export interface ToolChip {
  name: string;
  args: string;
  running: boolean;
  summary: string | null;
  error: boolean;
}

export interface ProposalView {
  proposal: Proposal;
  state: 'open' | 'applied' | 'dismissed';
}

/** An agent step's approval card: waiting, being sent, or answered (`expired`: the turn ended first). */
export type AgentApprovalState = 'pending' | 'sending' | AgentDecision | 'expired';

export interface AgentApprovalView {
  request: AgentApprovalRequest;
  state: AgentApprovalState;
}

/** A search or page the model suggested, shown as a card with the full address. */
export interface LinkView {
  kind: 'search' | 'link';
  url: string;
  title: string;
  query: string | null;
  segments: Segment[];
}

/** The model offers web research (or a search text needs confirming): "Research this" and agent mode. */
export interface ResearchOfferView {
  question: string;
  query: string;
  segments: Segment[];
  needsConfirm: boolean;
}

/** The agent step running now (one model call each, so the UI shows the progress). */
export interface AgentStepView {
  step: number;
  limit: number;
  action: string;
  site: string | null;
}

export interface PauseView {
  reason: PauseReason;
  steps: number;
  handOff: HandOff | null;
}

export interface TurnView {
  user: string;
  text: string;
  citations: Citation[];
  proposals: ProposalView[];
  tools: ToolChip[];
  approvals: AgentApprovalView[];
  links: LinkView[];
  offers: ResearchOfferView[];
  step: AgentStepView | null;
  paused: PauseView | null;
  notes: string[];
  error: string | null;
  cancelled: boolean;
  running: boolean;
  usage: Usage | null;
}

export function startTurn(user: string): TurnView {
  return { user, text: '', citations: [], proposals: [], tools: [], approvals: [], links: [], offers: [], step: null, paused: null, notes: [], error: null, cancelled: false, running: true, usage: null };
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

/** A turn that ended: nothing runs, and a card still waiting can no longer be answered. */
function ended(t: TurnView): TurnView {
  return {
    ...t,
    running: false,
    step: null,
    tools: t.tools.map((c) => ({ ...c, running: false })),
    approvals: t.approvals.map((a) => (a.state === 'pending' || a.state === 'sending' ? { ...a, state: 'expired' } : a)),
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
      const done: ToolChip = { name: e.name, args: t.tools[i]?.args ?? '', running: false, summary: e.summary, error: e.error };
      const tools = i >= 0 ? t.tools.map((c, j) => (j === i ? done : c)) : [...t.tools, done];
      return { ...t, tools };
    }
    case 'proposal':
      return { ...t, proposals: [...t.proposals, { proposal: e.proposal, state: 'open' }] };
    case 'linkSuggestion':
      return t.links.some((l) => l.url === e.url) ? t : { ...t, links: [...t.links, { kind: e.kind, url: e.url, title: e.title, query: e.query, segments: e.segments }] };
    case 'researchOffer':
      return t.offers.some((o) => o.query === e.query) ? t : { ...t, offers: [...t.offers, { question: e.question, query: e.query, segments: e.segments, needsConfirm: e.needsConfirm }] };
    case 'agentApproval':
      return { ...t, approvals: [...t.approvals, { request: e.request, state: 'pending' }] };
    case 'agentStep':
      return { ...t, step: { step: e.step, limit: e.limit, action: e.action, site: e.site } };
    case 'agentPaused':
      return { ...t, paused: { reason: e.reason, steps: e.steps, handOff: e.handOff }, step: null };
    case 'done':
      return {
        ...ended(t),
        text: e.text,
        citations: e.citations,
        notes: e.notes,
        usage: e.usage,
      };
    case 'error':
      return {
        ...ended(t),
        error: e.cancelled ? null : e.message,
        cancelled: e.cancelled,
      };
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

/**
 * What goes with a question (the composer's "Context" menu): the selected events and the current view, the current
 * view only (filter, time range, instance, open view), or nothing.
 */
export type ContextMode = 'selection' | 'view' | 'none';

/** The parts of the context a mode sends. */
export function contextToggles(mode: ContextMode): ContextToggles {
  switch (mode) {
    case 'selection':
      return ALL_CONTEXT;
    case 'view':
      return { ...ALL_CONTEXT, selection: false };
    case 'none':
      return { filter: false, range: false, selection: false, instance: false, view: false };
  }
}

/** The mode in effect: the user's choice, else the selected events when there are some, else the current view. */
export function effectiveContextMode(chosen: ContextMode | null, src: ContextSources): ContextMode {
  return chosen ?? (src.selectedIds.length ? 'selection' : 'view');
}

/** The label of a context mode ("Selected events (2)", "Current view", "None"). */
export function contextModeLabel(mode: ContextMode, selected: number): string {
  switch (mode) {
    case 'selection':
      return $localize`:@@ai.chatModel.contextMode.selection:Selected events (${selected}:count:)`;
    case 'view':
      return $localize`:@@ai.chatModel.contextMode.view:Current view`;
    case 'none':
      return $localize`:Context mode that sends only the question@@ai.chatModel.contextMode.none:None`;
  }
}

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

/** "Selected: 2 events (#4, #7)" (first three ids, then an ellipsis). */
function selectedLabel(ids: readonly number[]): string {
  const n = ids.length;
  const list = `#${ids.slice(0, 3).join(', #')}${n > 3 ? ', …' : ''}`;
  return plural(n, {
    one: $localize`:@@ai.chatModel.context.selected.one:Selected: ${n}:count: event (${list}:ids:)`,
    other: $localize`:@@ai.chatModel.context.selected.other:Selected: ${n}:count: events (${list}:ids:)`,
  });
}

/** One line per part of the context, for the chip that shows what is sent. */
export function describeContext(src: ContextSources, fmt: (ms: number) => string): { key: keyof ContextToggles; label: string; available: boolean }[] {
  const n = src.selectedIds.length;
  return [
    {
      key: 'filter',
      label: src.ql?.trim() ? $localize`:@@ai.chatModel.context.filter:Filter: ${src.ql.trim()}:query:` : $localize`:@@ai.chatModel.context.filterNone:Filter: none`,
      available: !!src.ql?.trim(),
    },
    {
      key: 'range',
      label: src.range
        ? $localize`:@@ai.chatModel.context.time:Time: ${fmt(src.range.from)}:from: – ${fmt(src.range.to)}:to:`
        : $localize`:@@ai.chatModel.context.timeAll:Time: whole workspace`,
      available: !!src.range,
    },
    { key: 'selection', label: n ? selectedLabel(src.selectedIds) : $localize`:@@ai.chatModel.context.selectedNone:Selected: none`, available: n > 0 },
    {
      key: 'instance',
      label: src.instance ? $localize`:@@ai.chatModel.context.instance:Instance: ${src.instance}:name:` : $localize`:@@ai.chatModel.context.instanceAll:Instance: all`,
      available: !!src.instance,
    },
    { key: 'view', label: src.view ? $localize`:@@ai.chatModel.context.view:View: ${src.view}:name:` : $localize`:@@ai.chatModel.context.viewNone:View: none`, available: !!src.view },
  ];
}

/** Human summary of a tool call for its chip ("run_query" → "Query"). */
export function toolLabel(name: string): string {
  const known: Record<string, string> = {
    run_query: $localize`:@@ai.chatModel.tool.runQuery:Query`,
    get_events: $localize`:@@ai.chatModel.tool.getEvents:Read events`,
    neighbours: $localize`:@@ai.chatModel.tool.neighbours:Surrounding lines`,
    field_stats: $localize`:@@ai.chatModel.tool.fieldStats:Field statistics`,
    check_query: $localize`:@@ai.chatModel.tool.checkQuery:Check query`,
    overview: $localize`:@@ai.chatModel.tool.overview:Health check`,
    exception_groups: $localize`:@@ai.chatModel.tool.exceptionGroups:Exceptions`,
    patterns: $localize`:@@ai.chatModel.tool.patterns:Patterns`,
    moment: $localize`:@@ai.chatModel.tool.moment:Moment`,
    anomalies: $localize`:@@ai.chatModel.tool.anomalies:Anomalies`,
    suggest_link: $localize`:@@ai.chatModel.tool.suggestLink:Suggest a link`,
    browser_read: $localize`:@@ai.chatModel.tool.browserRead:Read the browser page`,
    browser_search: $localize`:@@ai.chatModel.tool.browserSearch:Search in the browser`,
    browser_navigate: $localize`:@@ai.chatModel.tool.browserNavigate:Open a page`,
    browser_click: $localize`:@@ai.chatModel.tool.browserClick:Click`,
    browser_type: $localize`:@@ai.chatModel.tool.browserType:Type`,
    browser_scroll: $localize`:@@ai.chatModel.tool.browserScroll:Scroll`,
    browser_back: $localize`:@@ai.chatModel.tool.browserBack:Go back`,
    propose_regex: $localize`:@@ai.chatModel.tool.proposeRegex:Propose regex`,
    propose_query: $localize`:@@ai.chatModel.tool.proposeQuery:Propose query`,
    propose_parser: $localize`:@@ai.chatModel.tool.proposeParser:Propose parser`,
    propose_pipeline: $localize`:@@ai.chatModel.tool.proposePipeline:Propose pipeline`,
    propose_markers: $localize`:@@ai.chatModel.tool.proposeMarkers:Propose markers`,
    propose_extractors: $localize`:@@ai.chatModel.tool.proposeExtractors:Propose extractors`,

  };
  return known[name] ?? name.replace(/_/g, ' ');
}
