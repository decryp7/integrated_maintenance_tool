/**
 * A tiny, safe Markdown subset for model answers: paragraphs, headings, bullet and numbered lists, fenced
 * code, inline code, bold and italics. The result is plain data rendered with text bindings, never HTML,
 * so nothing in the model's text (or in log lines it quotes) can inject markup.
 *
 * Evidence becomes links: "event 123" / "#123" for cited records, backticked queries the engine accepted
 * as citations, and URLs of cited web pages.
 */
import { Citation } from '../ai.types';

export type Inline =
  | { kind: 'text'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'strong'; text: string }
  | { kind: 'em'; text: string }
  | { kind: 'record'; text: string; id: number; verified: boolean }
  | { kind: 'query'; text: string; ql: string }
  | { kind: 'web'; text: string; url: string; title: string };

export type Block =
  | { kind: 'p'; inlines: Inline[] }
  | { kind: 'h'; level: number; inlines: Inline[] }
  | { kind: 'ul' | 'ol'; items: Inline[][] }
  | { kind: 'code'; text: string };

/** Answers longer than this are cut (a runaway model must not freeze the view). */
export const MAX_RENDER_CHARS = 60_000;

interface Links {
  records: Map<number, boolean>;
  queries: Set<string>;
  web: Map<string, string>;
}

function linksOf(citations: readonly Citation[]): Links {
  const l: Links = { records: new Map(), queries: new Set(), web: new Map() };
  for (const c of citations) {
    if (c.kind === 'record') l.records.set(c.id, c.verified);
    else if (c.kind === 'query') l.queries.add(c.ql.trim());
    else if (c.kind === 'web' && /^https:\/\//i.test(c.url)) l.web.set(c.url, c.title);
  }
  return l;
}

const RECORD_REF = /\b(?:events?|records?|ids?)\s+#?(\d+)(?:(?:\s*,\s*|\s+and\s+|\s*&\s*)#?(\d+))*|#(\d+)/gi;
const NUMBER = /\d+/g;
const URL = /https:\/\/[^\s)<>"']+/g;

/** Splits plain text into text and link pieces (record references and cited URLs). */
function linkify(text: string, links: Links): Inline[] {
  type Hit = { start: number; end: number; inline: Inline };
  const hits: Hit[] = [];
  for (const m of text.matchAll(URL)) {
    const url = m[0].replace(/[.,;:!?]+$/, '');
    const title = links.web.get(url);
    if (title !== undefined) hits.push({ start: m.index, end: m.index + url.length, inline: { kind: 'web', text: url, url, title } });
  }
  if (links.records.size) {
    for (const m of text.matchAll(RECORD_REF)) {
      const start = m.index;
      if (hits.some((h) => start < h.end && start + m[0].length > h.start)) continue;
      // Each number of the reference is its own link ("events 4 and 7" → two links; "#12" keeps its "#").
      for (const n of m[0].matchAll(NUMBER)) {
        const id = Number(n[0]);
        const verified = links.records.get(id);
        if (verified === undefined) continue;
        const s = start + n.index;
        const from = text[s - 1] === '#' ? s - 1 : s;
        hits.push({ start: from, end: s + n[0].length, inline: { kind: 'record', text: text.slice(from, s + n[0].length), id, verified } });
      }
    }
  }
  hits.sort((a, b) => a.start - b.start);
  const out: Inline[] = [];
  let at = 0;
  for (const h of hits) {
    if (h.start < at) continue;
    if (h.start > at) out.push({ kind: 'text', text: text.slice(at, h.start) });
    out.push(h.inline);
    at = h.end;
  }
  if (at < text.length) out.push({ kind: 'text', text: text.slice(at) });
  return out;
}

const INLINE = /(`[^`\n]+`)|(\*\*[^*\n]+\*\*)|(__[^_\n]+__)|(\*[^*\s][^*\n]*\*)|(\b_[^_\s][^_\n]*_\b)/g;

/** Inline Markdown of one line or paragraph. */
export function parseInline(text: string, citations: readonly Citation[] = [], links = linksOf(citations)): Inline[] {
  const out: Inline[] = [];
  let at = 0;
  for (const m of text.matchAll(INLINE)) {
    if (m.index > at) out.push(...linkify(text.slice(at, m.index), links));
    const tok = m[0];
    if (m[1]) {
      const code = tok.slice(1, -1);
      out.push(links.queries.has(code.trim()) ? { kind: 'query', text: code, ql: code.trim() } : { kind: 'code', text: code });
    } else if (m[2] || m[3]) out.push({ kind: 'strong', text: tok.slice(2, -2) });
    else out.push({ kind: 'em', text: tok.slice(1, -1) });
    at = m.index + tok.length;
  }
  if (at < text.length) out.push(...linkify(text.slice(at), links));
  return merge(out);
}

/** Joins adjacent text pieces. */
function merge(list: Inline[]): Inline[] {
  const out: Inline[] = [];
  for (const i of list) {
    const last = out.at(-1);
    if (i.kind === 'text' && last?.kind === 'text') out[out.length - 1] = { kind: 'text', text: last.text + i.text };
    else if (i.kind !== 'text' || i.text) out.push(i);
  }
  return out;
}

const BULLET = /^\s*[-*+•]\s+(.*)$/;
const NUMBERED = /^\s*\d{1,3}[.)]\s+(.*)$/;
const HEADING = /^(#{1,4})\s+(.*)$/;
const FENCE = /^\s*```/;

/** Parses an answer (possibly still streaming: an unclosed code fence is shown as code). */
export function parseMarkdown(source: string, citations: readonly Citation[] = []): Block[] {
  const text = source.length > MAX_RENDER_CHARS ? `${source.slice(0, MAX_RENDER_CHARS)}…` : source;
  const links = linksOf(citations);
  const blocks: Block[] = [];
  let para: string[] = [];
  let list: { kind: 'ul' | 'ol'; items: Inline[][] } | null = null;
  const flushPara = () => {
    if (para.length) blocks.push({ kind: 'p', inlines: parseInline(para.join(' '), citations, links) });
    para = [];
  };
  const flushList = () => {
    if (list) blocks.push(list);
    list = null;
  };
  const lines = text.replace(/\r\n?/g, '\n').split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (FENCE.test(line)) {
      flushPara();
      flushList();
      const code: string[] = [];
      i++;
      while (i < lines.length && !FENCE.test(lines[i])) code.push(lines[i++]);
      blocks.push({ kind: 'code', text: code.join('\n') });
      continue;
    }
    if (!line.trim()) {
      flushPara();
      flushList();
      continue;
    }
    const h = HEADING.exec(line);
    if (h) {
      flushPara();
      flushList();
      blocks.push({ kind: 'h', level: h[1].length, inlines: parseInline(h[2], citations, links) });
      continue;
    }
    const b = BULLET.exec(line);
    const n = b ? null : NUMBERED.exec(line);
    if (b || n) {
      flushPara();
      const kind = b ? 'ul' : 'ol';
      if (list && list.kind !== kind) flushList();
      list ??= { kind, items: [] };
      list.items.push(parseInline((b ?? n)![1], citations, links));
      continue;
    }
    if (list && /^\s{2,}\S/.test(line)) {
      // A continuation line of the last list item.
      const items: Inline[][] = list.items;
      items[items.length - 1] = merge([...items[items.length - 1], { kind: 'text', text: ' ' }, ...parseInline(line.trim(), citations, links)]);
      continue;
    }
    flushList();
    para.push(line.trim());
  }
  flushPara();
  flushList();
  return blocks;
}

/** "Not found in the logs" answers are shown neutrally (not as an error or a finding). */
export function isNotFound(text: string): boolean {
  return /\bnot found in the logs\b/i.test(text);
}
