/**
 * Wire contract of the internal web browser (camelCase JSON), mirrored by the Rust DTOs in
 * `app/src-tauri/src/browser` and `crates/imt-ai/src/browser`. The chat events of read-along, suggested links
 * and agent mode are part of `ChatEvent` in `../ai/ai.types.ts`.
 */

export type SearchEngine = 'google' | 'duckDuckGo' | 'bing' | 'stackOverflow' | 'custom';

/** Every engine in the order the engine lists them (`SearchEngine::ALL`: Google, the default, first). */
export const SEARCH_ENGINES: readonly SearchEngine[] = ['google', 'duckDuckGo', 'bing', 'stackOverflow', 'custom'];

/** The page a window's browser shows. */
export interface PageState {
  open: boolean;
  url: string;
  title: string;
  loading: boolean;
}

/** `browser_state`. */
export interface BrowserStateDto {
  page: PageState;
  /** The engine from Settings ▸ Web browser. */
  engine: SearchEngine;
  /** A custom search address is set (the Custom engine can be used). */
  customEngine: boolean;
  /** The page shows in the Browser panel (Windows, macOS); false: in a window of its own (Linux). */
  docked: boolean;
}

/** What `browser_open` opens: the address box's text (a page or a search), or a search. */
export type OpenTarget =
  | { kind: 'address'; text: string; engine?: SearchEngine | null }
  | { kind: 'search'; query: string; engine?: SearchEngine | null };

/** Who wrote a search text: the model drafted it, or the engine built it (no model, or its draft was unusable). */
export type QuerySource = 'model' | 'engine';

/** `browser_event_query`: the search text for an event, drafted by the model or built by the engine, redacted. */
export interface EventQuery {
  query: string;
  /** What was taken out for the user's privacy ("2 file paths", "1 value from your logs"). */
  removed: string[];
  source: QuerySource;
}

export type NavCommand = 'back' | 'forward' | 'reload' | 'stop';

// `browser_place`: where the Browser panel's page area is, and whether the page may show there (ADR 0024).
// Generated from `browser::geometry`.
export type { CssRect, Placement } from '../core/generated/browser';

/** A key IMT took from the page (`key` event): Ctrl/⌘+L, Esc, or one of IMT's shortcuts to replay. */
export type PageKey =
  | { action: 'focusAddress' }
  | { action: 'returnFocus' }
  | { action: 'shortcut'; key: string; ctrl: boolean; shift: boolean; alt: boolean; meta: boolean };

/** A page handed to a chat ("Add page to AI context"), without its text. */
export interface SourceInfo {
  title: string;
  url: string;
  chars: number;
  truncated: boolean;
  removed: string[];
}

/** A chat's browser access (`browser_chat_status` and the `chatStatus` event). */
export interface ChatBrowserStatus {
  sessionId: string;
  readAlong: boolean;
  agent: boolean;
  stepsUsed: number;
  stepLimit: number;
  /** The step budget is spent: Continue grants another batch. */
  paused: boolean;
  /** Sites this chat may visit without asking. */
  sites: string[];
  /** The model may be too small for multi-step browsing (a hint only). */
  smallModel: boolean;
  modelParamsB: number | null;
  /** The AI researches the web by itself when a question needs it (Settings ▸ Web browser). */
  researchAuto: boolean;
}

export type NoticeKind = 'blocked' | 'download' | 'popup' | 'agentBlocked';

/** Who drives the window's browser now (`browser_driving` and the `driving` event): a chat's agent mode, or research. */
export type AiDriver = { kind: 'agent'; sessionId: string } | { kind: 'research' };

/** `imt://browser`, to the window that owns the browser and its pop-out panels. */
export type BrowserEvent =
  | ({ type: 'state' } & PageState)
  | { type: 'notice'; kind: NoticeKind; url: string; message: string }
  | { type: 'sources'; sessionId: string; sources: SourceInfo[] }
  | ({ type: 'chatStatus' } & ChatBrowserStatus)
  /** The AI started (`by`) or stopped (`null`) driving the browser. */
  | { type: 'driving'; by: AiDriver | null }
  /** The browser opened or loaded a page: show the Browser panel (`activate`: as the active tab). */
  | { type: 'reveal'; activate: boolean }
  | ({ type: 'key' } & PageKey);

/** The user's answer on an agent approval card. */
export type AgentDecision = 'allow' | 'allowSite' | 'deny';

/** What the leak guard marked: looks private, a value from the logs, or a piece of a log line. */
export type LeakKind = 'private' | 'logValue' | 'logText';

export interface Segment {
  text: string;
  leak: LeakKind | null;
}

export type AskKind = 'newSite' | 'leavingData' | 'typeText' | 'submitForm' | 'pressButton';

export interface AgentAsk {
  kind: AskKind;
  site: string | null;
  url: string | null;
  element: string | null;
  text: string | null;
  submit: boolean;
  canAllowSite: boolean;
}

export interface AgentApprovalRequest {
  requestId: number;
  ask: AgentAsk;
  urlSegments: Segment[];
  textSegments: Segment[];
}

export type HandOff = 'captcha' | 'login' | 'payment';
export type PauseReason = 'stepCap' | 'handBack' | 'stopped';
