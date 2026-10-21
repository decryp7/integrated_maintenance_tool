import { mockJobs } from '../jobs/mock-jobs';
/**
 * The internal browser for the browser build (dev and UI tests): a pretend page per IMT window, pages handed
 * to chats, and the per-chat read-along and agent switches the scripted AI mock plays with. It mimics the
 * contract of `app/src-tauri/src/browser` only; the engine (`crates/imt-ai/src/browser`) is the reference.
 */
import { Unlisten } from '../windowing/window-events';
import { BrowserBackend, ChatBrowserBackend } from './browser-backend';
import {
  AgentDecision,
  AiDriver,
  BrowserEvent,
  BrowserStateDto,
  ChatBrowserStatus,
  EventQuery,
  NavCommand,
  OpenTarget,
  Placement,
  PageState,
  SearchEngine,
  SourceInfo,
} from './browser.types';

const TEMPLATES: Record<Exclude<SearchEngine, 'custom'>, string> = {
  google: 'https://www.google.com/search?q={q}',
  duckDuckGo: 'https://duckduckgo.com/?q={q}',
  bing: 'https://www.bing.com/search?q={q}',
  stackOverflow: 'https://stackoverflow.com/search?q={q}',
};

/** A mock search page address, like the engine builds it. */
export function mockSearchUrl(engine: SearchEngine, query: string): string {
  const t = engine === 'custom' ? TEMPLATES.google : TEMPLATES[engine];
  return t.replace('{q}', encodeURIComponent(query.trim()).replace(/%20/g, '+'));
}

function titleFor(url: string): string {
  try {
    const u = new URL(url);
    const q = u.searchParams.get('q');
    return q ? `${q} at ${u.hostname.replace(/^www\./, '')}` : `${u.hostname.replace(/^www\./, '')}${u.pathname === '/' ? '' : u.pathname}`;
  } catch {
    return url;
  }
}

/** The shared pretend world: one page, chats' pages and switches, and waiting approvals. */
export class MockWeb {
  page: PageState = { open: false, url: '', title: '', loading: false };
  /** Settings ▸ Web browser ▸ Search engine (the engine's default: Google). */
  engine: SearchEngine = 'google';
  readonly history: string[] = [];
  at = -1;
  readonly handlers = new Set<(e: BrowserEvent) => void>();
  readonly sources = new Map<string, SourceInfo[]>();
  readonly statuses = new Map<string, ChatBrowserStatus>();
  readonly suggested = new Map<string, Set<string>>();
  readonly approvals = new Map<string, (d: AgentDecision) => void>();
  /** Addresses "opened" in the system browser (the browser build never leaves the page). */
  readonly external: string[] = [];
  /** The step limit of a new task (Settings ▸ Web browser). */
  stepLimit = 25;
  smallModel = false;
  /** Settings ▸ Web browser ▸ "Let the AI research the web automatically" (`?mockResearch=off` turns it off). */
  researchAuto = typeof window === 'undefined' || new URLSearchParams(window.location.search).get('mockResearch') !== 'off';
  /** "Open and let AI read it" hands the page to the chat; the mock AI backend sets this to ask the follow-up. */
  continueChat: ((sessionId: string, title: string) => Promise<number>) | null = null;

  /** The Browser panel's last `browser_place` (the page webview's place in the desktop app); e2e tests read it from
   * `window.__imtMockBrowserView`. */
  placement: Placement | null = null;

  /** Who drives the pretend browser (`?mockDriving=agent` or `research` starts with one, for the banner's e2e). */
  driver: AiDriver | null = mockDrivingParam();

  emit(e: BrowserEvent): void {
    for (const h of this.handlers) h(e);
  }

  /** The AI takes (or gives back) the browser; announced only when it changes, like the shell. */
  drive(by: AiDriver | null): void {
    if (JSON.stringify(by) === JSON.stringify(this.driver)) return;
    this.driver = by;
    this.emit({ type: 'driving', by });
  }

  /** Loads a page and asks the window to show the Browser panel (`activate`: the user's own; the AI's do not). */
  show(url: string, activate = true): void {
    this.history.splice(this.at + 1);
    this.history.push(url);
    this.at = this.history.length - 1;
    this.setPage(url);
    this.emit({ type: 'reveal', activate });
  }

  setPage(url: string): void {
    this.page = { open: true, url, title: titleFor(url), loading: false };
    this.emit({ type: 'state', ...this.page });
  }

  status(sessionId: string): ChatBrowserStatus {
    let s = this.statuses.get(sessionId);
    if (!s) {
      s = { sessionId, readAlong: false, agent: false, stepsUsed: 0, stepLimit: this.stepLimit, paused: false, sites: [], smallModel: this.smallModel, modelParamsB: this.smallModel ? 4 : 9, researchAuto: this.researchAuto };
      this.statuses.set(sessionId, s);
    }
    return s;
  }

  update(sessionId: string, patch: Partial<ChatBrowserStatus>): ChatBrowserStatus {
    const s = { ...this.status(sessionId), ...patch };
    this.statuses.set(sessionId, s);
    this.emit({ type: 'chatStatus', ...s });
    return s;
  }

  /** The pages for the turn that starts now (they go with it). */
  takeSources(sessionId: string): SourceInfo[] {
    const list = this.sources.get(sessionId) ?? [];
    this.sources.delete(sessionId);
    if (list.length) this.emit({ type: 'sources', sessionId, sources: [] });
    return list;
  }

  /** Waits for the user's answer on approval card `requestId` of a chat. */
  ask(sessionId: string, requestId: number): Promise<AgentDecision> {
    return new Promise((resolve) => this.approvals.set(`${sessionId}#${requestId}`, resolve));
  }

  denyAll(sessionId: string): void {
    for (const [k, resolve] of this.approvals) {
      if (k.startsWith(`${sessionId}#`)) {
        this.approvals.delete(k);
        resolve('deny');
      }
    }
  }

  forget(sessionId: string): void {
    this.denyAll(sessionId);
    this.sources.delete(sessionId);
    this.statuses.delete(sessionId);
    this.suggested.delete(sessionId);
  }
}

function mockDrivingParam(): AiDriver | null {
  if (typeof window === 'undefined') return null;
  const p = new URLSearchParams(window.location.search).get('mockDriving');
  return p === 'research' ? { kind: 'research' } : p === 'agent' ? { kind: 'agent', sessionId: 'mock-chat' } : null;
}

let world: MockWeb | null = null;

export function mockWeb(): MockWeb {
  return (world ??= new MockWeb());
}

/** A fresh world (unit tests). */
export function resetMockWeb(): MockWeb {
  world = new MockWeb();
  return world;
}

const later = <T>(f: () => T): Promise<T> =>
  new Promise((resolve, reject) =>
    setTimeout(() => {
      try {
        resolve(f());
      } catch (e) {
        reject(e);
      }
    }, 0),
  );

export class MockBrowserBackend implements BrowserBackend, ChatBrowserBackend {
  constructor(private readonly w: MockWeb = mockWeb()) {}

  state() {
    return later((): BrowserStateDto => ({ page: { ...this.w.page }, engine: this.w.engine, customEngine: false, docked: true }));
  }
  open(target: OpenTarget) {
    return later(() => {
      const engine = target.engine ?? this.w.engine;
      let url: string;
      if (target.kind === 'search') {
        if (!target.query.trim()) throw new Error('the search is empty');
        url = mockSearchUrl(engine, target.query);
      } else {
        const t = target.text.trim();
        if (!t) throw new Error('the address is empty');
        if (/^(javascript|file|data):/i.test(t)) throw new Error(`only http and https pages open in the browser (not ${t.split(':')[0]}:)`);
        url = /^https?:\/\//i.test(t) ? t : /^[a-z0-9-]+(\.[a-z0-9-]+)+(\/\S*)?$/i.test(t) ? `https://${t}` : mockSearchUrl(engine, t);
      }
      this.w.show(url);
      return url;
    });
  }
  eventQuery(recordId: number) {
    return later((): EventQuery => {
      if (recordId < 0) throw new Error('nothing to search for: there is no such event');
      return { query: 'System.TimeoutException The operation has timed out', removed: ['1 value from your logs'], source: 'model' };
    });
  }
  /** Like the shell: the page shows while the panel says it may and its area has a size (there is no native page here). */
  place(placement: Placement) {
    return later(() => {
      const { rect } = placement;
      if (!Number.isFinite(placement.scale) || placement.scale < 0.25 || placement.scale > 8) throw new Error(`the scale ${placement.scale} is out of range`);
      const shown = placement.visible && !!rect && rect.width * placement.scale >= 1 && rect.height * placement.scale >= 1;
      this.w.placement = placement;
      if (typeof window !== 'undefined') (window as unknown as { __imtMockBrowserView?: unknown }).__imtMockBrowserView = { ...placement, shown };
      return shown;
    });
  }
  nav(command: NavCommand) {
    return later(() => {
      if (!this.w.page.open) throw new Error('the browser is not open');
      if (command === 'back' && this.w.at > 0) this.w.setPage(this.w.history[--this.w.at] ?? '');
      if (command === 'forward' && this.w.at < this.w.history.length - 1) this.w.setPage(this.w.history[++this.w.at] ?? '');
      if (command === 'reload') this.w.setPage(this.w.page.url);
    });
  }
  close() {
    return later(() => {
      const was = this.w.page.open;
      this.w.page = { open: false, url: '', title: '', loading: false };
      this.w.emit({ type: 'state', ...this.w.page });
      return was;
    });
  }
  openExternal(url: string) {
    return later(() => {
      if (url !== this.w.page.url) throw new Error('refused: only the page shown or a window it asked to open can be opened');
      this.w.external.push(url);
    });
  }
  capture(sessionId: string) {
    return later(() => {
      if (!this.w.page.open) throw new Error('the browser is not open; open it from the Browser panel first');
      const list = (this.w.sources.get(sessionId) ?? []).filter((s) => s.url !== this.w.page.url);
      list.push({ title: this.w.page.title, url: this.w.page.url, chars: 1834, truncated: false, removed: [] });
      this.w.sources.set(sessionId, list.slice(-3));
      const sources = [...(this.w.sources.get(sessionId) ?? [])];
      this.w.emit({ type: 'sources', sessionId, sources });
      return sources;
    });
  }
  sources(sessionId: string) {
    return later(() => [...(this.w.sources.get(sessionId) ?? [])]);
  }
  discardSources(sessionId: string) {
    return later(() => {
      this.w.sources.delete(sessionId);
      this.w.emit({ type: 'sources', sessionId, sources: [] });
      return [];
    });
  }
  status(sessionId: string) {
    return later(() => ({ ...this.w.status(sessionId) }));
  }
  setReadAlong(sessionId: string, on: boolean) {
    return later(() => this.w.update(sessionId, { readAlong: on }));
  }
  setAgent(sessionId: string, on: boolean) {
    return later(() => {
      if (!on) this.w.denyAll(sessionId);
      if (!on) this.stopDriving(sessionId);
      return this.w.update(sessionId, on ? { agent: true, stepsUsed: 0, stepLimit: this.w.stepLimit, paused: false } : { agent: false });
    });
  }
  stop(sessionId: string) {
    return later(() => {
      this.w.denyAll(sessionId);
      this.stopDriving(sessionId);
      return this.w.update(sessionId, { agent: false });
    });
  }
  continueTask(sessionId: string) {
    return later(() => {
      const s = this.w.status(sessionId);
      if (!s.agent) throw new Error('agent mode is off in this chat');
      return this.w.update(sessionId, { stepLimit: s.stepsUsed + this.w.stepLimit, paused: false });
    });
  }
  respond(sessionId: string, requestId: number, decision: AgentDecision) {
    return later(() => {
      const key = `${sessionId}#${requestId}`;
      const resolve = this.w.approvals.get(key);
      this.w.approvals.delete(key);
      resolve?.(decision);
      return !!resolve;
    });
  }
  openSuggested(sessionId: string, url: string) {
    return later(() => {
      if (!this.w.suggested.get(sessionId)?.has(url)) throw new Error('refused: this link was not suggested in this chat');
      this.w.show(url);
    });
  }
  async openAndRead(sessionId: string, url: string): Promise<number> {
    await this.openSuggested(sessionId, url);
    await this.capture(sessionId);
    return this.w.continueChat ? this.w.continueChat(sessionId, this.w.page.title) : 0;
  }
  researchStop() {
    return later(() => {
      if (this.w.driver?.kind === 'research') this.w.drive(null);
      return mockJobs().cancelWhere((j) => j.kind === 'research');
    });
  }
  driving() {
    return later(() => this.w.driver);
  }
  private stopDriving(sessionId: string): void {
    const d = this.w.driver;
    if (d?.kind === 'agent' && d.sessionId === sessionId) this.w.drive(null);
  }
  onEvent(handler: (e: BrowserEvent) => void): Promise<Unlisten> {
    this.w.handlers.add(handler);
    return Promise.resolve(() => this.w.handlers.delete(handler));
  }
}

export function mockBrowserBackend(): MockBrowserBackend {
  return new MockBrowserBackend(mockWeb());
}
