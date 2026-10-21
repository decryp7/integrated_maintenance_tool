import { inject } from '@angular/core';
import { patchState, signalStore, withHooks, withMethods, withState } from '@ngrx/signals';
import { BrowserBackend } from './browser-backend';
import { AiDriver, BrowserEvent, NavCommand, NoticeKind, OpenTarget, PageKey, PageState, SearchEngine } from './browser.types';

/** A notice of the browser shown in the Browser panel until dismissed. */
export interface BrowserNotice {
  id: number;
  kind: NoticeKind;
  url: string;
  message: string;
}

interface BrowserStoreState {
  page: PageState;
  /** The engine the panel searches with: null until `browser_state` answers (the backend then uses Settings ▸ Web browser). */
  engine: SearchEngine | null;
  customEngine: boolean;
  /** The page shows in the panel (false on Linux: a window of its own). */
  docked: boolean;
  /** Who drives the browser now ("AI is browsing" in the panel header and on its tab); null: the user. */
  driver: AiDriver | null;
  /** A `driving` event came: it wins over a slower first answer of `browser_driving`. */
  driverHeard: boolean;
  notices: BrowserNotice[];
  /** The last request that failed (shown inline with Retry). */
  error: string | null;
  busy: boolean;
  loaded: boolean;
  /** The last key IMT took from the page (Ctrl+L, Esc, a shortcut), numbered so the same key twice is seen twice. */
  key: { seq: number; key: PageKey } | null;
}

/** Most notices kept in the panel. */
export const MAX_NOTICES = 5;

const initial: BrowserStoreState = {
  page: { open: false, url: '', title: '', loading: false },
  engine: null,
  customEngine: false,
  docked: true,
  driver: null,
  driverHeard: false,
  notices: [],
  error: null,
  busy: false,
  loaded: false,
  key: null,
};

function message(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return $localize`:@@browser.store.failed:The browser did not answer.`;
}

/**
 * The Browser panel's state for this window's browser: the page shown, the engine, notices (blocked
 * navigations, downloads, popups) and the keys taken from the page. The shell owns the browser; this keeps what the
 * panel shows.
 */
export const BrowserStore = signalStore(
  { providedIn: 'root' },
  withState<BrowserStoreState>(initial),
  withMethods((store, backend = inject(BrowserBackend)) => {
    let nextId = 1;
    /** Runs a request; a failure is kept for the inline error with Retry. */
    const run = async <T>(f: () => Promise<T>): Promise<T | null> => {
      patchState(store, { busy: true, error: null });
      try {
        return await f();
      } catch (e) {
        patchState(store, { error: message(e) });
        return null;
      } finally {
        patchState(store, { busy: false });
      }
    };
    let lastRequest: (() => Promise<unknown>) | null = null;
    const remember = <T>(f: () => Promise<T>) => {
      lastRequest = f;
      return run(f);
    };
    return {
      onEvent(e: BrowserEvent): void {
        if (e.type === 'state') {
          const { type: _type, ...page } = e;
          patchState(store, { page });
        } else if (e.type === 'notice') {
          const n: BrowserNotice = { id: nextId++, kind: e.kind, url: e.url, message: e.message };
          patchState(store, (s) => ({ notices: [...s.notices.filter((x) => !(x.kind === n.kind && x.url === n.url)), n].slice(-MAX_NOTICES) }));
        } else if (e.type === 'driving') {
          patchState(store, { driver: e.by, driverHeard: true });
        } else if (e.type === 'key') {
          const { type: _type, ...key } = e;
          patchState(store, (s) => ({ key: { seq: (s.key?.seq ?? 0) + 1, key } }));
        }
      },

      async load(): Promise<void> {
        const s = await remember(() => backend.state());
        if (s) patchState(store, { page: s.page, engine: store.loaded() ? (store.engine() ?? s.engine) : s.engine, customEngine: s.customEngine, docked: s.docked, loaded: true });
        const d = await backend.driving().catch(() => undefined);
        if (d !== undefined && !store.driverHeard()) patchState(store, { driver: d });
      },

      setEngine(engine: SearchEngine): void {
        patchState(store, { engine });
      },

      /** Opens the address box's text (a page or a search) with the panel's engine. */
      async go(text: string): Promise<boolean> {
        return (await remember(() => backend.open({ kind: 'address', text, engine: store.engine() }))) !== null;
      },

      async open(target: OpenTarget): Promise<boolean> {
        return (await remember(() => backend.open(target))) !== null;
      },

      async nav(command: NavCommand): Promise<void> {
        await remember(() => backend.nav(command));
      },

      async close(): Promise<void> {
        await remember(() => backend.close());
      },

      async openExternal(url: string): Promise<boolean> {
        return (await remember(() => backend.openExternal(url))) !== null;
      },

      dismiss(id: number): void {
        patchState(store, (s) => ({ notices: s.notices.filter((n) => n.id !== id) }));
      },

      /** Retry for the inline error. */
      async retry(): Promise<void> {
        if (lastRequest) await run(lastRequest);
      },

      dismissError(): void {
        patchState(store, { error: null });
      },
    };
  }),
  withHooks((store, backend = inject(BrowserBackend)) => {
    let off: (() => void) | undefined;
    let destroyed = false;
    return {
      onInit() {
        void backend
          .onEvent((e) => store.onEvent(e))
          .then((u) => (destroyed ? u() : (off = u)))
          .catch(() => undefined);
      },
      onDestroy() {
        destroyed = true;
        off?.();
      },
    };
  }),
);

export type BrowserStoreType = InstanceType<typeof BrowserStore>;
