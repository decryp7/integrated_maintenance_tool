/**
 * Wire contract of windows, the system tray and background work (camelCase JSON), mirroring the
 * Rust DTOs in `app/src-tauri`. Commands act on the calling window (resolved in Rust from its label).
 */

/** An app window as the shell sees it. */
export interface WindowInfo {
  label: string;
  title: string;
  focused: boolean;
  visible: boolean;
}

export type TaskKind = 'liveTail' | 'processMonitor' | 'perfmon' | 'job' | 'other';

/** Work that keeps running while windows are closed (shown in the tray and the status bar). */
export interface BackgroundTask {
  id: string;
  label: string;
  kind: TaskKind;
  /** Label of the window that owns the task, or null for app-wide work. */
  window: string | null;
  /** 0..1, or null when the work has no measurable progress. */
  progress: number | null;
  stoppable: boolean;
  /** Epoch milliseconds. */
  startedAt: number;
}

/** The answer to "Close Debrief?" when the last window closes. */
export type CloseChoice = 'quit' | 'tray';

/** The answer to the open-ask: open the waiting files here, in a new window, or drop them (dismissed). */
export type OpenChoice = 'here' | 'newWindow' | 'cancel';

/** Files from another app are waiting; the user picks the window (paths stay in Rust). */
export interface OpenAsk {
  count: number;
  /** File names only. */
  names: string[];
}

/** Events emitted to the owning window only. */
export const CLOSE_REQUESTED_EVENT = 'debrief://close-requested';
export const OPEN_ASK_EVENT = 'debrief://open-ask';
export const CAPTURE_EVENT = 'debrief://capture';
export const LIVE_STOPPED_EVENT = 'debrief://live-stopped';
/** Global events. */
export const BACKGROUND_TASKS_EVENT = 'debrief://background-tasks';
export const WINDOWS_CHANGED_EVENT = 'debrief://windows-changed';
