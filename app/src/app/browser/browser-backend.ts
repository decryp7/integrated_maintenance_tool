import { inject, Injectable, Injector } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { listenHere, PayloadListenFn, Unlisten } from '../windowing/window-events';
import { cancelActiveJobs } from '../jobs/job-cancel';
import {
  AgentDecision,
  AiDriver,
  BrowserEvent,
  BrowserStateDto,
  ChatBrowserStatus,
  EventQuery,
  NavCommand,
  OpenTarget,
  SourceInfo,
} from './browser.types';

export const BROWSER_EVENT = 'imt://browser';

/**
 * The browser window of this IMT window, as the Web panel drives it. Injected as a class token: the Tauri
 * commands in the desktop app, the mock in the browser build (dev and UI tests).
 */
@Injectable({ providedIn: 'root', useFactory: () => createBrowserBackends().browser })
export abstract class BrowserBackend {
  abstract state(): Promise<BrowserStateDto>;
  /** Opens the browser (or loads the page in it); resolves with the address opened. */
  abstract open(target: OpenTarget): Promise<string>;
  /** The engine-built, redacted search text for an event of the loaded logs. */
  abstract eventQuery(recordId: number): Promise<EventQuery>;
  abstract nav(command: NavCommand): Promise<void>;
  /** Closes the browser window; false when it was not open. */
  abstract close(): Promise<boolean>;
  /** Stops the research reading pages in this window's browser; resolves with how many stopped. */
  abstract researchStop(): Promise<number>;
  /** Who drives this window's browser now (null: nobody, the browser is the user's). */
  abstract driving(): Promise<AiDriver | null>;
  /** Opens the page shown, or a popup it asked for, in the system browser (anything else is refused). */
  abstract openExternal(url: string): Promise<void>;
  abstract onEvent(handler: (e: BrowserEvent) => void): Promise<Unlisten>;
}

/** What a chat may do with the browser: hand-off pages, read-along, suggested links and agent mode. */
@Injectable({ providedIn: 'root', useFactory: () => createBrowserBackends().chat })
export abstract class ChatBrowserBackend {
  /** "Add page to AI context": the visible page, redacted, for the chat's next question. */
  abstract capture(sessionId: string): Promise<SourceInfo[]>;
  abstract sources(sessionId: string): Promise<SourceInfo[]>;
  abstract discardSources(sessionId: string): Promise<SourceInfo[]>;
  abstract status(sessionId: string): Promise<ChatBrowserStatus>;
  abstract setReadAlong(sessionId: string, on: boolean): Promise<ChatBrowserStatus>;
  abstract setAgent(sessionId: string, on: boolean): Promise<ChatBrowserStatus>;
  /** Stops agent mode at once (a waiting approval is denied). */
  abstract stop(sessionId: string): Promise<ChatBrowserStatus>;
  /** Grants another batch of steps after the step cap. */
  abstract continueTask(sessionId: string): Promise<ChatBrowserStatus>;
  abstract respond(sessionId: string, requestId: number, decision: AgentDecision): Promise<boolean>;
  /** Opens a link the chat suggested in the internal browser (other links are refused). */
  abstract openSuggested(sessionId: string, url: string): Promise<void>;
  /** "Open and let AI read it": opens a suggested link, adds the page to the chat and lets the chat go on (a new turn; its job). */
  abstract openAndRead(sessionId: string, url: string): Promise<number>;
  abstract onEvent(handler: (e: BrowserEvent) => void): Promise<Unlisten>;
}

const isEvent = (p: unknown): p is BrowserEvent => !!p && typeof p === 'object' && typeof (p as { type?: unknown }).type === 'string';

/** Talks to the Rust shell (contract: `browser.types.ts`); `call` is Tauri's `invoke`, tests pass fakes. */
export class TauriBrowserBackend implements BrowserBackend, ChatBrowserBackend {
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly listen: PayloadListenFn = listenHere,
  ) {}

  state() {
    return this.call<BrowserStateDto>('browser_state');
  }
  open(target: OpenTarget) {
    return this.call<string>('browser_open', { target });
  }
  eventQuery(recordId: number) {
    return this.call<EventQuery>('browser_event_query', { recordId });
  }
  nav(command: NavCommand) {
    return this.call<void>('browser_nav', { command });
  }
  close() {
    return this.call<boolean>('browser_close');
  }
  /** Stops the window's research by cancelling its jobs (the banner clears with them). */
  researchStop() {
    return cancelActiveJobs(this.call, (j) => j.kind === 'research');
  }
  driving() {
    return this.call<AiDriver | null>('browser_driving');
  }
  openExternal(url: string) {
    return this.call<void>('browser_open_external', { url });
  }
  capture(sessionId: string) {
    return this.call<SourceInfo[]>('browser_capture', { sessionId });
  }
  sources(sessionId: string) {
    return this.call<SourceInfo[]>('browser_sources', { sessionId });
  }
  discardSources(sessionId: string) {
    return this.call<SourceInfo[]>('browser_discard_sources', { sessionId });
  }
  status(sessionId: string) {
    return this.call<ChatBrowserStatus>('browser_chat_status', { sessionId });
  }
  setReadAlong(sessionId: string, on: boolean) {
    return this.call<ChatBrowserStatus>('browser_set_read_along', { sessionId, on });
  }
  setAgent(sessionId: string, on: boolean) {
    return this.call<ChatBrowserStatus>('browser_set_agent', { sessionId, on });
  }
  stop(sessionId: string) {
    return this.call<ChatBrowserStatus>('browser_agent_stop', { sessionId });
  }
  continueTask(sessionId: string) {
    return this.call<ChatBrowserStatus>('browser_agent_continue', { sessionId });
  }
  respond(sessionId: string, requestId: number, decision: AgentDecision) {
    return this.call<boolean>('browser_agent_respond', { sessionId, requestId, decision });
  }
  openSuggested(sessionId: string, url: string) {
    return this.call<void>('browser_open_suggested', { sessionId, url });
  }
  openAndRead(sessionId: string, url: string) {
    return this.call<number>('browser_open_and_read', { sessionId, url });
  }
  onEvent(handler: (e: BrowserEvent) => void) {
    return this.listen(BROWSER_EVENT, (p) => {
      if (isEvent(p)) handler(p);
    });
  }
}

type Both = BrowserBackend & ChatBrowserBackend;

let shared: { injector: Injector; both: Both } | null = null;

/**
 * One adapter serves both tokens (the Tauri commands, or the mock loaded lazily in a browser), behind the
 * activity and failure proxy like every backend. Call in an injection context.
 */
export function createBrowserBackends(): { browser: BrowserBackend; chat: ChatBrowserBackend } {
  const injector = inject(Injector);
  if (shared?.injector !== injector) {
    const target: Both = isTauri() ? new TauriBrowserBackend() : lazyMock();
    shared = { injector, both: withFailureLogging<Both>(target, inject(Logger), { onFailure: reportBackendFailures() }) };
  }
  return { browser: shared.both, chat: shared.both };
}

/** Defers loading the mock until the first call. */
function lazyMock(): Both {
  let loaded: Promise<Both> | null = null;
  const get = () => (loaded ??= import('./mock-browser-backend').then((m) => m.mockBrowserBackend()));
  return new Proxy({} as Both, {
    get(_t, name) {
      if (typeof name !== 'string' || name === 'then' || name.startsWith('ng')) return undefined;
      return (...args: unknown[]) =>
        get().then((b) => {
          const fn = (b as unknown as Record<string, (...a: unknown[]) => Promise<unknown>>)[name];
          if (!fn) throw new TypeError(`${name} is not a function`);
          return fn.apply(b, args);
        });
    },
  });
}
