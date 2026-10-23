/**
 * The activity log's wire shapes. A `Breadcrumb` is the same JSON as Rust's `activity::Breadcrumb`
 * (app/src-tauri/src/activity/breadcrumbs.rs).
 */

/**
 * How the user started an operation. `shortcut`: a key combination; `keyboard`: typing or Enter in a field;
 * `pointer`: a click on something that is not a button (a grid row, the timeline); `program`: no user input
 * led to it (restore, timers, another window).
 */
export type Trigger =
  | 'menu'
  | 'native-menu'
  | 'palette'
  | 'shortcut'
  | 'keyboard'
  | 'context-menu'
  | 'button'
  | 'pointer'
  | 'drag-drop'
  | 'dialog'
  | 'program'
  | 'unknown';

export const TRIGGERS: readonly Trigger[] = ['menu', 'native-menu', 'palette', 'shortcut', 'keyboard', 'context-menu', 'button', 'pointer', 'drag-drop', 'dialog', 'program', 'unknown'];

export type Outcome = 'ok' | 'cancelled' | 'error';

/**
 * Parameters of an entry. Values must already be safe: ids, counts, names of things, enum values. Text
 * the user typed goes through `safeText`, paths through `safePath`; keys that name secrets are masked.
 */
export type ActivityParams = Record<string, string | number | boolean | null | undefined>;

/** One activity entry as kept in the ring and attached to errors. */
export interface Breadcrumb {
  time: string;
  operation: string;
  trigger: string;
  operationId?: string;
  outcome?: Outcome;
  errorCode?: string;
  durationMs?: number;
  view?: string;
  window: string;
  /** Absent when empty (the engine leaves it out). */
  params?: Record<string, string>;
}

/** `SourceContext` of every activity entry. */
export const ACTIVITY_CONTEXT = 'Debrief.Activity';
/** The message template of user operations. */
export const USER_TEMPLATE = 'User {Operation} via {Trigger}';
/** IPC header carrying the operation id (Rust: `activity::OP_HEADER`). */
export const OP_HEADER = 'debrief-op-id';
/** Entries kept per window. */
export const RING_SIZE = 200;
/** Entries attached to an error. */
export const BREADCRUMBS_PER_ERROR = 30;
