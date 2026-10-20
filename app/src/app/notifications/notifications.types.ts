// Wire contract of the notification centre (`notifications_*` commands, `imt://notifications` and
// `imt://notifications-open` events; camelCase JSON), mirrored by
// app/src-tauri/src/notification_centre/{centre,persist,mod}.rs. Change both sides together.

export type NoticeSource = 'job' | 'liveAlert' | 'import' | 'plugin' | 'ai' | 'update' | 'session';

export const NOTICE_SOURCES: readonly NoticeSource[] = ['job', 'liveAlert', 'import', 'plugin', 'ai', 'update', 'session'];

export type NoticeSeverity = 'info' | 'success' | 'warning' | 'error';

export type NoticeScope = 'app' | 'project';

/** What a message's button does; the UI maps each to the existing feature. */
export type NoticeAction =
  | { type: 'viewJob'; jobId: number }
  | { type: 'retryJob'; jobId: number }
  | { type: 'reviewProposals'; jobId: number }
  | { type: 'resetPlugin'; pluginId: string }
  | { type: 'setUpFormat'; path: string }
  | { type: 'fixTimestamps'; path: string; format: string | null }
  | { type: 'resetAi' }
  | { type: 'installUpdate' }
  | { type: 'showAlert'; alertId: string }
  | { type: 'showJobs' };

export interface NoticeLink {
  window: string | null;
  jobId: number | null;
  recordId: number | null;
  pluginId: string | null;
  path: string | null;
}

export interface Notice {
  id: number;
  source: NoticeSource;
  severity: NoticeSeverity;
  title: string;
  body: string;
  /** Epoch ms of the latest occurrence. */
  at: number;
  firstAt: number;
  /** How many times it happened (the same dedupe key merges). */
  count: number;
  read: boolean;
  primary: NoticeAction | null;
  secondary: NoticeAction | null;
  link: NoticeLink;
  dedupeKey: string | null;
  scope: NoticeScope;
}

/** The window's messages (its project's and the app's), newest first. */
export interface NoticesSnapshot {
  notices: Notice[];
  unread: number;
}

/** `imt://notifications`: the list, and the message just posted (previewed as a toast). */
export interface NoticesEvent extends NoticesSnapshot {
  posted: Notice | null;
}

/** The project's messages (UI state key `notifications`). */
export interface NoticesRecord {
  version: number;
  notices: unknown[];
}

export interface RestoredNotices {
  snapshot: NoticesSnapshot;
  warning: string | null;
}

export const EMPTY_NOTICES: NoticesSnapshot = { notices: [], unread: 0 };
