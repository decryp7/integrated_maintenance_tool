import type { WindowingBackend } from './windowing-backend';
import type { Unlisten } from './window-events';
import { BackgroundTask, CloseChoice, OpenAsk, OpenChoice, WindowInfo } from './windowing.types';

/** localStorage key of the mock's "when the last window is closed" preference. */
export const MOCK_ON_CLOSE_KEY = 'debrief.mock.onClose';
/** localStorage key of the counter that names new mock windows (shared by all tabs of the origin). */
export const MOCK_WINDOW_SEQ_KEY = 'debrief.mock.windowSeq';

type OnClose = 'ask' | CloseChoice;

/** What the mock needs from the browser; tests pass a fake. */
export interface MockWindowHost {
  href(): string;
  open(url: string): Window | null;
  close(): void;
  setTitle(title: string): void;
  title(): string;
  hasFocus(): boolean;
  storage(): Storage | null;
}

/* v8 ignore start: thin wrapper over browser globals */
export const browserHost: MockWindowHost = {
  href: () => window.location.href,
  open: (url) => window.open(url, '_blank'),
  close: () => window.close(),
  setTitle: (title) => void (document.title = title),
  title: () => document.title,
  hasFocus: () => document.hasFocus(),
  storage: () => {
    try {
      return window.localStorage;
    } catch {
      return null;
    }
  },
};
/* v8 ignore stop */

/** A host without side effects (UI tests): popups are recorded, storage is in memory. */
export function memoryHost(href = 'http://localhost/'): MockWindowHost & { popups: string[]; closed: number } {
  const data = new Map<string, string>();
  const storage = { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v) } as Storage;
  let title = '';
  const host = {
    popups: [] as string[],
    closed: 0,
    href: () => href,
    open: (url: string) => (host.popups.push(url), null),
    close: () => void host.closed++,
    setTitle: (t: string) => void (title = t),
    title: () => title,
    hasFocus: () => true,
    storage: () => storage,
  };
  return host;
}

/** Tasks seeded with `?mock-tasks`. */
export function seededTasks(now = Date.now()): BackgroundTask[] {
  return [
    { id: 'live-main', label: 'Live tail — main', kind: 'liveTail', window: 'main', progress: null, stoppable: true, startedAt: now - 60_000 },
    { id: 'monitor', label: 'Process monitor', kind: 'processMonitor', window: null, progress: null, stoppable: true, startedAt: now - 120_000 },
    { id: 'symbols', label: 'Downloading symbols', kind: 'job', window: null, progress: 0.4, stoppable: false, startedAt: now - 5_000 },
  ];
}

/** The open-ask seeded with `?mock-open-ask=<comma-separated names>` (at most 20 names). */
export function seededOpenAsk(param: string | null): OpenAsk | null {
  if (param === null) return null;
  const names = param
    .split(',')
    .map((n) => n.trim())
    .filter(Boolean)
    .slice(0, 20);
  return names.length ? { count: names.length, names } : null;
}

class Handlers<T> {
  private readonly list = new Set<(value: T) => void>();
  add(handler: (value: T) => void): Promise<Unlisten> {
    this.list.add(handler);
    return Promise.resolve(() => void this.list.delete(handler));
  }
  emit(value: T): void {
    for (const h of [...this.list]) h(value);
  }
  get size(): number {
    return this.list.size;
  }
}

/**
 * Browser-build stand-in for windows and the tray. Each window is a tab (`?window=<label>`) with its
 * own MockBackend, so workspaces are independent. Browser development and UI tests only.
 */
export class MockWindowingBackend implements WindowingBackend {
  /** Every call, for assertions. */
  readonly calls: string[] = [];
  private readonly ownLabel: string;
  private readonly opened = new Map<string, Window | null>();
  private tasks: BackgroundTask[];
  private readonly closeRequested = new Handlers<void>();
  private readonly openAsk = new Handlers<OpenAsk>();
  private readonly capture = new Handlers<void>();
  private readonly liveStopped = new Handlers<void>();
  private readonly tasksChanged = new Handlers<BackgroundTask[]>();
  private readonly windowsChanged = new Handlers<WindowInfo[]>();

  /** An open-ask shown once the UI listens (`?mock-open-ask=a.log,b.log`). */
  private seededAsk: OpenAsk | null;

  constructor(private readonly host: MockWindowHost = browserHost) {
    const q = new URL(host.href()).searchParams;
    this.ownLabel = q.get('window') || 'main';
    this.tasks = q.has('mock-tasks') ? seededTasks() : [];
    this.seededAsk = seededOpenAsk(q.get('mock-open-ask'));
  }

  async label(): Promise<string> {
    return this.ownLabel;
  }

  async newWindow(): Promise<WindowInfo> {
    this.calls.push('newWindow');
    const label = `w-${this.nextSeq()}`;
    const url = new URL(this.host.href());
    url.searchParams.set('window', label);
    this.opened.set(label, this.host.open(url.toString()));
    const info: WindowInfo = { label, title: `Debrief — ${label}`, focused: true, visible: true };
    this.windowsChanged.emit(await this.listWindows());
    return info;
  }

  async closeWindow(): Promise<void> {
    this.calls.push('closeWindow');
    const onClose = this.onClose();
    if (onClose === 'ask') this.closeRequested.emit();
    else if (onClose === 'quit') this.host.close();
    // 'tray': the window would hide into the tray; nothing to show in a browser tab.
    // Visible to end-to-end tests, which cannot read `calls` (and need a sign that the close was handled).
    console.info(`[mock windowing] close_window ${onClose}`);
  }

  async quitApp(): Promise<void> {
    this.calls.push('quitApp');
    // Visible to end-to-end tests, which cannot read `calls`.
    console.info('[mock windowing] quit_app');
  }

  async listWindows(): Promise<WindowInfo[]> {
    const own: WindowInfo = { label: this.ownLabel, title: this.host.title() || 'Debrief', focused: this.host.hasFocus(), visible: true };
    const others = [...this.opened]
      .filter(([, w]) => !w?.closed)
      .map(([label]): WindowInfo => ({ label, title: `Debrief — ${label}`, focused: false, visible: true }));
    return [own, ...others];
  }

  async focusWindow(label: string): Promise<void> {
    this.calls.push(`focusWindow:${label}`);
    this.opened.get(label)?.focus();
  }

  async setWindowTitle(title: string): Promise<void> {
    this.host.setTitle(title);
  }

  async restoreWindows(): Promise<number> {
    this.calls.push('restoreWindows');
    return 0;
  }

  async resolveClose(choice: CloseChoice, remember: boolean): Promise<void> {
    this.calls.push(`resolveClose:${choice}:${remember}`);
    if (remember) this.write(choice);
    if (choice === 'quit') this.host.close();
  }

  async resolveOpen(choice: OpenChoice): Promise<void> {
    this.calls.push(`resolveOpen:${choice}`);
    // Visible to end-to-end tests, which cannot read `calls`.
    console.info(`[mock windowing] resolve_open ${choice}`);
  }

  async backgroundTasks(): Promise<BackgroundTask[]> {
    return structuredClone(this.tasks);
  }

  async stopBackgroundTask(id: string): Promise<void> {
    this.calls.push(`stopBackgroundTask:${id}`);
    const task = this.tasks.find((t) => t.id === id);
    if (!task) throw `there is no background task "${id}"`;
    if (!task.stoppable) throw `${task.label} cannot be stopped`;
    this.tasks = this.tasks.filter((t) => t.id !== id);
    if (task.kind === 'liveTail' && task.window === this.ownLabel) this.liveStopped.emit();
    this.tasksChanged.emit(structuredClone(this.tasks));
  }

  onCloseRequested(handler: () => void): Promise<Unlisten> {
    return this.closeRequested.add(handler);
  }
  onOpenAsk(handler: (ask: OpenAsk) => void): Promise<Unlisten> {
    const added = this.openAsk.add(handler);
    const seeded = this.seededAsk;
    this.seededAsk = null;
    if (seeded) setTimeout(() => this.openAsk.emit(seeded));
    return added;
  }
  onCapture(handler: () => void): Promise<Unlisten> {
    return this.capture.add(handler);
  }
  onLiveStopped(handler: () => void): Promise<Unlisten> {
    return this.liveStopped.add(handler);
  }
  onBackgroundTasks(handler: (tasks: BackgroundTask[]) => void): Promise<Unlisten> {
    return this.tasksChanged.add(handler);
  }
  onWindowsChanged(handler: (windows: WindowInfo[]) => void): Promise<Unlisten> {
    return this.windowsChanged.add(handler);
  }

  /** Simulates the shell's events (tests and manual checks). */
  emitOpenAsk(ask: OpenAsk): void {
    this.openAsk.emit(ask);
  }
  emitCapture(): void {
    this.capture.emit();
  }
  emitLiveStopped(): void {
    this.liveStopped.emit();
  }
  /** Number of live subscriptions (to check unsubscribing). */
  get listeners(): number {
    return [this.closeRequested, this.openAsk, this.capture, this.liveStopped, this.tasksChanged, this.windowsChanged].reduce((n, h) => n + h.size, 0);
  }

  private onClose(): OnClose {
    const v = this.read(MOCK_ON_CLOSE_KEY);
    return v === 'quit' || v === 'tray' ? v : 'ask';
  }

  private nextSeq(): number {
    const n = (Number(this.read(MOCK_WINDOW_SEQ_KEY)) || 1) + 1;
    try {
      this.host.storage()?.setItem(MOCK_WINDOW_SEQ_KEY, String(n));
    } catch {
      /* storage unavailable */
    }
    return n;
  }

  private read(key: string): string | null {
    try {
      return this.host.storage()?.getItem(key) ?? null;
    } catch {
      return null;
    }
  }

  private write(choice: CloseChoice): void {
    try {
      this.host.storage()?.setItem(MOCK_ON_CLOSE_KEY, choice);
    } catch {
      /* storage unavailable: the choice is not remembered */
    }
  }
}
