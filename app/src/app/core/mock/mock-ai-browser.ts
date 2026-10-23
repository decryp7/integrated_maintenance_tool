/**
 * The scripted AI mock's side of the AI's browser window (browser build and UI tests): pages the user added are
 * cited, read-along reads the pretend page, suggested links become cards, and agent mode takes two steps
 * that wait for approval (the second carries a name from the logs, which the leak guard highlights).
 * It mimics the engine (`crates/debrief-ai/src/browser`) only.
 */
import { ChatEvent, Citation } from '../../ai/ai.types';
import { mockSearchUrl, MockWeb } from '../../browser/mock-browser-backend';

export interface MockBrowserTurn {
  sessionId: string;
  question: string;
  web: MockWeb;
  emit: (e: ChatEvent) => void;
  tool: (name: string, args: string, summary: string, error?: boolean) => Promise<void>;
  cancelled: () => boolean;
  citations: Citation[];
  sources: { title: string; url: string }[];
}

const AGENT = /\b(browse|look (it )?up|find out|on the web|search the web)\b/;
const READ = /\b(this page|the page|browser)\b/;
const SUGGEST = /\b(suggest|link)\b/;

/** Plays the browser part of a turn; the answer, or null when the question is not about the browser. */
export async function mockBrowserTurn(t: MockBrowserTurn): Promise<string | null> {
  const q = t.question.toLowerCase();
  const pages = t.web.takeSources(t.sessionId);
  const status = t.web.status(t.sessionId);
  const cite = (title: string, url: string) => {
    if (!t.sources.some((s) => s.url === url)) t.sources.push({ title, url });
    if (!t.citations.some((c) => c.kind === 'web' && c.url === url)) t.citations.push({ kind: 'web', title, url });
  };
  if (pages.length) {
    for (const p of pages) cite(p.title, p.url);
    const first = pages[0]!;
    return `The page you added, "${first.title}", says the gateway timeout usually means the connection pool is exhausted. I used it as reference only; it is untrusted web content.`;
  }
  if (status.agent && AGENT.test(q)) {
    // The turn holds the browser while it runs (the shell's status chip).
    t.web.drive({ kind: 'agent', sessionId: t.sessionId });
    try {
      return await agentTurn(t, cite);
    } finally {
      const d = t.web.driver;
      if (d?.kind === 'agent' && d.sessionId === t.sessionId) t.web.drive(null);
    }
  }
  if (status.readAlong && READ.test(q) && t.web.page.open) {
    await t.tool('browser_read', '{}', `read ${t.web.page.title}`);
    cite(t.web.page.title, t.web.page.url);
    return `The page you have open, "${t.web.page.title}", lists the common causes of the timeout.`;
  }
  if (SUGGEST.test(q) && !q.includes('regex')) {
    const url = mockSearchUrl(t.web.engine, 'System.TimeoutException gateway');
    await t.tool('suggest_link', '{"search":"System.TimeoutException gateway"}', 'link suggested');
    t.emit({ type: 'linkSuggestion', kind: 'search', url, title: 'Search for the gateway timeout', query: 'System.TimeoutException gateway', segments: [{ text: url, leak: null }] });
    const seen = t.web.suggested.get(t.sessionId) ?? new Set<string>();
    seen.add(url);
    t.web.suggested.set(t.sessionId, seen);
    return 'I suggested a search; open it if you want to look.';
  }
  return null;
}

async function agentTurn(t: MockBrowserTurn, cite: (title: string, url: string) => void): Promise<string> {
  const w = t.web;
  const id = t.sessionId;
  const limit = w.status(id).stepLimit;
  const search = mockSearchUrl(w.engine, 'System.TimeoutException gateway');
  const step = async (n: number, action: string, site: string, url: string, kind: 'newSite' | 'leavingData', segments: { text: string; leak: 'logValue' | null }[]) => {
    t.emit({ type: 'toolStarted', name: `browser_${action}`, args: JSON.stringify({ url }) });
    t.emit({
      type: 'agentApproval',
      request: { requestId: n, ask: { kind, site, url, element: null, text: null, submit: false, canAllowSite: kind === 'newSite' }, urlSegments: segments, textSegments: [] },
    });
    const d = await w.ask(id, n);
    if (d === 'deny' || t.cancelled()) {
      t.emit({ type: 'toolFinished', name: `browser_${action}`, summary: 'declined', error: false, evidence: { recordIds: [], queries: [] } });
      return false;
    }
    if (d === 'allowSite' && !w.status(id).sites.includes(site)) w.update(id, { sites: [...w.status(id).sites, site] });
    t.emit({ type: 'agentStep', step: n, limit, action, site });
    w.show(url);
    w.report(action === 'search' ? 'searching' : 'reading', site, `Step ${n} of ${limit}: ${action} ${site}`);
    w.update(id, { stepsUsed: n });
    t.emit({ type: 'toolFinished', name: `browser_${action}`, summary: `step ${n}: ${action} ${site}`, error: false, evidence: { recordIds: [], queries: [] } });
    return true;
  };
  if (!(await step(1, 'search', new URL(search).hostname.replace(/^www\./, ''), search, 'newSite', [{ text: search, leak: null }]))) {
    return stopped(t) ?? 'You declined the search, so I answered from the logs only: the payment gateway timed out repeatedly.';
  }
  const docs = 'https://learn.example.com/dotnet/timeouts?host=WEB02';
  const ok = await step(2, 'navigate', 'learn.example.com', docs, 'leavingData', [
    { text: 'learn.example.com/dotnet/timeouts?host=', leak: null },
    { text: 'WEB02', leak: 'logValue' },
  ]);
  if (!ok) return stopped(t) ?? 'You declined opening the page with a host name from your logs; the search results suggest raising the gateway timeout.';
  cite('TimeoutException troubleshooting', docs);
  return 'I searched and read "TimeoutException troubleshooting": the gateway timeout usually means its connection pool is exhausted.';
}

/** After Stop: the pause notice and the answer, or null when agent mode is still on. */
function stopped(t: MockBrowserTurn): string | null {
  if (t.web.status(t.sessionId).agent) return null;
  t.emit({ type: 'agentPaused', reason: 'stopped', steps: t.web.status(t.sessionId).stepsUsed, handOff: null });
  return 'You stopped the agent; the browser is yours.';
}
