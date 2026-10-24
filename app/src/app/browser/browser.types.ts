/**
 * Wire contract of the AI's browser (camelCase JSON), mirrored by the Rust DTOs in `app/src-tauri/src/browser` and
 * `crates/debrief-ai/src/browser`. Research and the chat's browser modes drive a real Chromium window of their own over
 * DevTools; links open in the user's default browser. The chat events of read-along, suggested links and agent mode
 * are part of `ChatEvent` in `../ai/ai.types.ts`.
 */

export type SearchEngine = 'google' | 'duckDuckGo' | 'bing' | 'custom';

/** Every engine in the order the engine lists them (`SearchEngine::ALL`: Google, the default, first). */
export const SEARCH_ENGINES: readonly SearchEngine[] = ['google', 'duckDuckGo', 'bing', 'custom'];

/** What `browser_open` opens in the default browser: the address box's text (a page or a search), or a search. */
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

/**
 * How a chat uses the AI's browser window (the composer's "Browser" menu): not at all, read-along (the AI may read the page you
 * have open, never browsing by itself) or agent mode (the AI drives the browser step by step). Exclusive.
 */
export type ChatBrowserMode = 'off' | 'readAlong' | 'agent';

/** The mode a chat's status is in (agent mode wins over read-along). */
export function chatBrowserMode(s: Pick<ChatBrowserStatus, 'readAlong' | 'agent'> | null): ChatBrowserMode {
  if (s?.agent) return 'agent';
  return s?.readAlong ? 'readAlong' : 'off';
}

/** Who drives the window's browser now (`browser_driving` and the `driving` event): a chat's agent mode, or research. */
export type AiDriver = { kind: 'agent'; sessionId: string } | { kind: 'research' };

/** What the AI's browser window is doing (`activity` events): the window title reads "<product> AI — <activity>: <host>". */
export type BrowserActivityKind = 'reading' | 'searching' | 'waitingForYou';

/** Where a window's AI browser is in its life; the engine owns it (`lifecycle` events). */
export type BrowserLifecycleState = 'none' | 'launching' | 'ready' | 'closing' | 'gone';

/** Why the AI browser is gone: the user closed its window, it quit or crashed, its connection ended, Debrief ended it, or it did not start. */
export type BrowserGoneReason = 'windowClosed' | 'exited' | 'disconnected' | 'ended' | 'launchFailed';

/** `debrief://browser`, to the window that owns the browser and its pop-out panels. */
export type BrowserEvent =
  | { type: 'sources'; sessionId: string; sources: SourceInfo[] }
  | ({ type: 'chatStatus' } & ChatBrowserStatus)
  /** The AI started (`by`) or stopped (`null`) driving the browser. */
  | { type: 'driving'; by: AiDriver | null }
  /**
   * The AI browser moved in its life cycle; `kept` while research left it open because the user took part in it (offer
   * `closeResearchBrowser()`). Once gone, the next research opens a new one.
   */
  | { type: 'lifecycle'; state: BrowserLifecycleState; reason?: BrowserGoneReason; kept: boolean }
  /** A line like "Reading 3 of 5 results". */
  | { type: 'status'; text: string }
  | { type: 'activity'; activity: BrowserActivityKind; host: string; title: string };

/** Where the Chromium that research and the browser modes drive comes from. */
export type RuntimeSource = 'configured' | 'installed' | 'found' | 'none';

/** `browser_runtime_status` (and what install and remove return). */
export interface RuntimeStatus {
  /** A browser can be started now. */
  available: boolean;
  source: RuntimeSource;
  path: string | null;
  installedVersion: string | null;
  pinnedVersion: string;
  /** There is a pinned download for this platform. */
  canInstall: boolean;
}

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
