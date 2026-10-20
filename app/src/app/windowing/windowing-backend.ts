import { invoke } from '../core/activity/ipc';
import { inject, Injectable, Provider } from '@angular/core';
import { reportBackendFailures } from '../core/failure-reporter';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { currentWindowLabel, listenHere, PayloadListenFn, Unlisten } from './window-events';
import { parseOpenAsk, parseTasks, parseWindows } from './windowing-payloads';
import {
  BACKGROUND_TASKS_EVENT,
  BackgroundTask,
  CAPTURE_EVENT,
  CLOSE_REQUESTED_EVENT,
  CloseChoice,
  LIVE_STOPPED_EVENT,
  OPEN_ASK_EVENT,
  OpenAsk,
  OpenChoice,
  WindowInfo,
  WINDOWS_CHANGED_EVENT,
} from './windowing.types';
import { browserMocks } from '../core/mock-registry';

/** The real backend inside Tauri, the in-memory mock in a plain browser; failures are logged and reported. */
export function createWindowingBackend(): WindowingBackend {
  const real = isTauri() ? new TauriWindowingBackend() : new (browserMocks().MockWindowingBackend)();
  return withFailureLogging<WindowingBackend>(real, inject(Logger), { onFailure: reportBackendFailures() });
}

/**
 * Windows, the system tray and background work. Every command acts on the calling window.
 * Provided in root (so features that only need e.g. `restoreWindows` work without `provideWindowing`).
 */
@Injectable({ providedIn: 'root', useFactory: createWindowingBackend })
export abstract class WindowingBackend {
  /** This window's label. */
  abstract label(): Promise<string>;
  /** Opens and focuses a new, empty window with its own workspace. */
  abstract newWindow(): Promise<WindowInfo>;
  /** Asks to close this window; the shell applies the "on close" setting (and may emit close-requested). */
  abstract closeWindow(): Promise<void>;
  abstract listWindows(): Promise<WindowInfo[]>;
  abstract focusWindow(label: string): Promise<void>;
  abstract setWindowTitle(title: string): Promise<void>;
  /** After a crash restore, reopens the previous run's other windows; returns how many. */
  abstract restoreWindows(): Promise<number>;
  abstract resolveClose(choice: CloseChoice, remember: boolean): Promise<void>;
  /** Answers the open-ask; `cancel` drops the waiting files in Rust. */
  abstract resolveOpen(choice: OpenChoice): Promise<void>;
  abstract backgroundTasks(): Promise<BackgroundTask[]>;
  abstract stopBackgroundTask(id: string): Promise<void>;
  abstract onCloseRequested(handler: () => void): Promise<Unlisten>;
  abstract onOpenAsk(handler: (ask: OpenAsk) => void): Promise<Unlisten>;
  abstract onCapture(handler: () => void): Promise<Unlisten>;
  abstract onLiveStopped(handler: () => void): Promise<Unlisten>;
  abstract onBackgroundTasks(handler: (tasks: BackgroundTask[]) => void): Promise<Unlisten>;
  abstract onWindowsChanged(handler: (windows: WindowInfo[]) => void): Promise<Unlisten>;
}

/** Talks to the Rust shell. `call`, `listen` and `labelOf` are injectable for tests. */
export class TauriWindowingBackend extends WindowingBackend {
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly listen: PayloadListenFn = listenHere,
    private readonly labelOf: () => Promise<string> = currentWindowLabel,
  ) {
    super();
  }

  label() {
    return this.labelOf();
  }
  newWindow() {
    return this.call<WindowInfo>('new_window');
  }
  closeWindow() {
    return this.call<void>('close_window');
  }
  async listWindows() {
    return parseWindows(await this.call<unknown>('list_windows'));
  }
  focusWindow(label: string) {
    return this.call<void>('focus_window', { label });
  }
  setWindowTitle(title: string) {
    return this.call<void>('set_window_title', { title });
  }
  restoreWindows() {
    return this.call<number>('restore_windows');
  }
  resolveClose(choice: CloseChoice, remember: boolean) {
    return this.call<void>('resolve_close', { choice, remember });
  }
  resolveOpen(choice: OpenChoice) {
    return this.call<void>('resolve_open', { choice });
  }
  async backgroundTasks() {
    return parseTasks(await this.call<unknown>('background_tasks'));
  }
  stopBackgroundTask(id: string) {
    return this.call<void>('stop_background_task', { id });
  }
  onCloseRequested(handler: () => void) {
    return this.listen(CLOSE_REQUESTED_EVENT, () => handler());
  }
  onOpenAsk(handler: (ask: OpenAsk) => void) {
    return this.listen(OPEN_ASK_EVENT, (p) => {
      const ask = parseOpenAsk(p);
      if (ask) handler(ask);
    });
  }
  onCapture(handler: () => void) {
    return this.listen(CAPTURE_EVENT, () => handler());
  }
  onLiveStopped(handler: () => void) {
    return this.listen(LIVE_STOPPED_EVENT, () => handler());
  }
  onBackgroundTasks(handler: (tasks: BackgroundTask[]) => void) {
    return this.listen(BACKGROUND_TASKS_EVENT, (p) => handler(parseTasks(p)));
  }
  onWindowsChanged(handler: (windows: WindowInfo[]) => void) {
    return this.listen(WINDOWS_CHANGED_EVENT, (p) => handler(parseWindows(p)));
  }
}

export function provideWindowingBackend(): Provider {
  return { provide: WindowingBackend, useFactory: createWindowingBackend };
}
