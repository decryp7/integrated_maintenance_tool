import { InjectionToken } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { isTauri } from '../core/platform';
import { Rect } from '../dock/dock-geometry';
import { WorkArea } from '../popout/bounds';
import { currentWindowLabel, listenHere, Unlisten } from '../windowing/window-events';
import { ConfirmQuestion } from './dialog-session';

/** Wire contract of the dialog commands (camelCase JSON), mirroring `app/src-tauri/src/dialog_windows.rs`. */
export interface DialogOpenRequest {
  dialogId: string;
  instance?: string;
  title: string;
  modality: 'modal' | 'modeless';
  input?: unknown;
  width?: number;
  height?: number;
  minWidth?: number;
  minHeight?: number;
  bounds?: Rect;
}
export interface DialogInfo {
  label: string;
  dialogId: string;
  existing: boolean;
}
export interface DialogStart {
  dialogId: string;
  title: string;
  modality: 'modal' | 'modeless';
  input: unknown;
}
export interface DialogResultEvent {
  label: string;
  dialogId: string;
  result: unknown;
}
export interface DialogClosedEvent {
  label: string;
  dialogId: string;
  bounds: Rect | null;
}

export const DIALOG_RESULT_EVENT = 'debrief://dialog-result';
export const DIALOG_CLOSED_EVENT = 'debrief://dialog-closed';
export const DIALOG_CLOSE_REQUESTED_EVENT = 'debrief://dialog-close-requested';

/** Everything the dialog windows need from Tauri, in one injectable seam (tests pass fakes). */
export interface DialogIo {
  open(request: DialogOpenRequest): Promise<DialogInfo>;
  take(): Promise<DialogStart>;
  close(result: unknown): Promise<void>;
  listen(event: string, handler: (payload: unknown) => void): Promise<Unlisten>;
  /** Asks the dialog window `label` to close (it may refuse). */
  requestClose(label: string): Promise<void>;
  ask(question: ConfirmQuestion): Promise<boolean>;
  workAreas(): Promise<WorkArea[]>;
  windowLabel(): Promise<string>;
}

export const DIALOG_IO = new InjectionToken<DialogIo>('DIALOG_IO', { providedIn: 'root', factory: createDialogIo });

export function tauriDialogIo(): DialogIo {
  return {
    open: (request) => invoke<DialogInfo>('dialog_open', { request }),
    take: () => invoke<DialogStart>('dialog_take_input'),
    close: (result) => invoke<void>('dialog_close', { result: result === undefined ? null : result }),
    listen: listenHere,
    requestClose: async (label) => {
      const { emitTo } = await import('@tauri-apps/api/event');
      await emitTo(label, DIALOG_CLOSE_REQUESTED_EVENT, null);
    },
    ask: async (q) => {
      const { ask } = await import('@tauri-apps/plugin-dialog');
      return ask(q.message, { title: q.title, kind: q.danger ? 'warning' : 'info', okLabel: q.ok, cancelLabel: q.cancel });
    },
    workAreas: async () => {
      const { availableMonitors, primaryMonitor } = await import('@tauri-apps/api/window');
      const [monitors, primary] = await Promise.all([availableMonitors(), primaryMonitor()]);
      return monitors.map((m) => {
        const s = m.scaleFactor || 1;
        const wa = m.workArea ?? { position: m.position, size: m.size };
        return { name: m.name ?? null, primary: !!primary && primary.name === m.name, x: wa.position.x / s, y: wa.position.y / s, width: wa.size.width / s, height: wa.size.height / s };
      });
    },
    windowLabel: currentWindowLabel,
  };
}

export function createDialogIo(): DialogIo {
  return isTauri() ? tauriDialogIo() : browserDialogIo();
}

/** Key under which the browser build leaves a dialog window's answer (tests and tooling read it). */
export const BROWSER_DIALOG_RESULT_KEY = 'debrief.dialog.browserResult';

/**
 * The browser build has no dialog windows (the host shows dialogs in the page), but the dialog route can still be
 * opened by URL (`#/dialog?...&dialogInput=<json>`): this is for tests and measurements of the dialog window's own
 * bootstrap. The answer is left in `localStorage` and the tab closes.
 */
export function browserDialogIo(): DialogIo {
  const route = () => new URLSearchParams(location.hash.split('?')[1] ?? '');
  return {
    open: async () => {
      throw new Error('dialogs open in the page in the browser build');
    },
    take: async () => {
      const r = route();
      let input: unknown = {};
      try {
        input = JSON.parse(r.get('dialogInput') ?? '{}');
      } catch {
        /* no input */
      }
      return { dialogId: r.get('id') ?? '', title: r.get('title') ?? r.get('id') ?? '', modality: r.get('modality') === 'modal' ? 'modal' : 'modeless', input };
    },
    close: async (result) => {
      try {
        localStorage.setItem(BROWSER_DIALOG_RESULT_KEY, JSON.stringify({ result: result ?? null }));
      } catch {
        /* a convenience only */
      }
      window.close();
    },
    listen: async () => () => undefined,
    requestClose: async () => undefined,
    ask: async (q) => window.confirm(q.message),
    workAreas: async () => [],
    windowLabel: async () => route().get('label') ?? '',
  };
}
