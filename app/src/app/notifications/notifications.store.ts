import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { NotificationsBackend } from './notifications-backend';
import { EMPTY_NOTICES, Notice, NoticeSource, NoticesEvent, NoticesSnapshot } from './notifications.types';

/** Toasts shown at once (newest last). */
export const MAX_TOASTS = 3;
/** A toast goes away by itself after this long (the message stays in Notifications). */
export const TOAST_MS = 8000;

export type ReadFilter = 'all' | 'unread';

/**
 * A message of the UI's own in the centre (not the engine's), with an action that runs in the UI: e.g. "New folder?
 * Set up analysis…". It lives for the session; its id is negative.
 */
export interface LocalNotice {
  notice: Notice;
  actionLabel: string;
  run: () => void;
}

/** What a feature posts as a local message. `key` replaces an earlier message with the same key. */
export interface NewLocalNotice {
  key: string;
  source: NoticeSource;
  severity: Notice['severity'];
  title: string;
  body?: string;
  actionLabel: string;
  run: () => void;
}

interface NotificationsState {
  snapshot: NoticesSnapshot;
  /** Toast previews of messages just posted. */
  toasts: Notice[];
  sources: NoticeSource[];
  read: ReadFilter;
  /** The message to reveal in the view (an OS notification was clicked, or a toast's "Show in Notifications"). */
  focus: number | null;
  error: string | null;
  loaded: boolean;
  local: LocalNotice[];
}

const initial: NotificationsState = { snapshot: EMPTY_NOTICES, toasts: [], sources: [], read: 'all', focus: null, error: null, loaded: false, local: [] };

function message(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return $localize`:@@notifications.store.failed:The notification centre did not answer.`;
}

/** Sources with toasts of their own (live alerts have mute and show); their messages are not previewed again. */
const OWN_TOASTS: readonly NoticeSource[] = ['liveAlert'];

/**
 * The window's notification centre as the engine reports it: the list (dedupe, counts, caps and read state are
 * the engine's), the view's filters, and toast previews of new messages.
 */
export const NotificationsStore = signalStore(
  { providedIn: 'root' },
  withState<NotificationsState>(initial),
  withComputed((store) => {
    const all = computed(() => [...store.local().map((l) => l.notice), ...store.snapshot().notices]);
    const filtered = computed(() => {
      const sources = store.sources();
      return all().filter((n) => (!sources.length || sources.includes(n.source)) && (store.read() === 'all' || !n.read));
    });
    return {
      notices: filtered,
      unread: computed(() => store.snapshot().unread + store.local().filter((l) => !l.notice.read).length),
      /** The filtered messages grouped by source, in the order sources first appear (newest first). */
      groups: computed(() => {
        const groups = new Map<NoticeSource, Notice[]>();
        for (const n of filtered()) groups.set(n.source, [...(groups.get(n.source) ?? []), n]);
        return [...groups.entries()].map(([source, notices]) => ({ source, notices }));
      }),
    };
  }),
  withMethods((store, backend = inject(NotificationsBackend)) => {
    let last: (() => Promise<unknown>) | null = null;
    const timers = new Map<number, ReturnType<typeof setTimeout>>();
    let localIds = 0;
    const run = async <T>(f: () => Promise<T>): Promise<T | null> => {
      last = f;
      patchState(store, { error: null });
      try {
        return await f();
      } catch (e) {
        patchState(store, { error: message(e) });
        return null;
      }
    };
    const markLocal = (id: number) => patchState(store, (s) => ({ local: s.local.map((l) => (l.notice.id === id ? { ...l, notice: { ...l.notice, read: true } } : l)) }));
    const removeLocal = (id: number) => patchState(store, (s) => ({ local: s.local.filter((l) => l.notice.id !== id) }));
    const dismissToast = (id: number) => {
      clearTimeout(timers.get(id));
      timers.delete(id);
      patchState(store, (s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
    };
    return {
      apply(e: NoticesEvent): void {
        patchState(store, { snapshot: { notices: e.notices, unread: e.unread }, loaded: true });
        const p = e.posted;
        if (p && !p.read && !OWN_TOASTS.includes(p.source)) {
          patchState(store, (s) => ({ toasts: [...s.toasts.filter((t) => t.id !== p.id), p].slice(-MAX_TOASTS) }));
          clearTimeout(timers.get(p.id));
          timers.set(
            p.id,
            setTimeout(() => dismissToast(p.id), TOAST_MS),
          );
        }
      },

      /** Reads the list (Retry of a failed first load reads and applies it again). */
      async load(): Promise<void> {
        await run(async () => {
          const s = await backend.list();
          patchState(store, { snapshot: s, loaded: true });
          return s;
        });
      },

      markRead: (id: number) => (id < 0 ? Promise.resolve(markLocal(id)) : run(() => backend.markRead(id))),
      markAllRead: () => {
        patchState(store, (s) => ({ local: s.local.map((l) => ({ ...l, notice: { ...l.notice, read: true } })) }));
        return run(() => backend.markAllRead());
      },
      async dismiss(id: number): Promise<void> {
        dismissToast(id);
        if (id < 0) {
          removeLocal(id);
          return;
        }
        await run(() => backend.dismiss(id));
      },

      /** Posts (or replaces, by key) a message of the UI's own; it counts as unread until read, dismissed or acted on. */
      postLocal(n: NewLocalNotice): void {
        const now = Date.now();
        const dedupeKey = 'local:' + n.key;
        const notice: Notice = {
          id: -++localIds,
          source: n.source,
          severity: n.severity,
          title: n.title,
          body: n.body ?? '',
          at: now,
          firstAt: now,
          count: 1,
          read: false,
          primary: null,
          secondary: null,
          tertiary: null,
          link: { window: null, jobId: null, recordId: null, pluginId: null, path: null },
          dedupeKey,
          scope: 'project',
        };
        patchState(store, (s) => ({ local: [{ notice, actionLabel: n.actionLabel, run: n.run }, ...s.local.filter((l) => l.notice.dedupeKey !== dedupeKey)] }));
      },

      /** Removes the local message with this key, if any (what it offered no longer applies). */
      withdrawLocal(key: string): void {
        patchState(store, (s) => ({ local: s.local.filter((l) => l.notice.dedupeKey !== 'local:' + key) }));
      },

      /** The label of a local message's action; null for the engine's messages. */
      localAction(id: number): string | null {
        return store.local().find((l) => l.notice.id === id)?.actionLabel ?? null;
      },

      /** Runs a local message's action; the message is done. */
      runLocal(id: number): void {
        const item = store.local().find((l) => l.notice.id === id);
        if (!item) return;
        removeLocal(id);
        item.run();
      },

      dismissToast,

      /** A toast of the UI's own (not a message of the centre): e.g. "Explaining event #7 — running in Jobs". */
      showLocal(n: Pick<Notice, 'severity' | 'title'> & { body?: string; primary?: Notice['primary'] }): void {
        const now = Date.now();
        const toast: Notice = {
          id: -++localIds,
          source: 'job',
          severity: n.severity,
          title: n.title,
          body: n.body ?? '',
          at: now,
          firstAt: now,
          count: 1,
          read: false,
          primary: n.primary ?? null,
          secondary: null,
          tertiary: null,
          link: { window: null, jobId: null, recordId: null, pluginId: null, path: null },
          dedupeKey: null,
          scope: 'project',
        };
        patchState(store, (s) => ({ toasts: [...s.toasts, toast].slice(-MAX_TOASTS) }));
        timers.set(
          toast.id,
          setTimeout(() => dismissToast(toast.id), TOAST_MS),
        );
      },

      /** Keeps the toast open while the pointer or focus is on it. */
      holdToast(id: number): void {
        clearTimeout(timers.get(id));
        timers.delete(id);
      },

      toggleSource(source: NoticeSource): void {
        const s = store.sources();
        patchState(store, { sources: s.includes(source) ? s.filter((x) => x !== source) : [...s, source] });
      },

      clearSources(): void {
        patchState(store, { sources: [] });
      },

      setRead(read: ReadFilter): void {
        patchState(store, { read });
      },

      setFocus(id: number | null): void {
        patchState(store, { focus: id });
      },

      async retryRequest(): Promise<void> {
        if (last) await run(last);
      },

      dismissError(): void {
        patchState(store, { error: null });
      },
    };
  }),
  withHooks((store, backend = inject(NotificationsBackend)) => {
    const offs: (() => void)[] = [];
    let destroyed = false;
    const keep = (u: () => void) => (destroyed ? u() : offs.push(u));
    return {
      onInit() {
        void backend.onChange((e) => store.apply(e)).then(keep).catch(() => undefined);
        void backend
          .onOpen((id) => store.setFocus(id))
          .then(keep)
          .catch(() => undefined);
        void store.load();
      },
      onDestroy() {
        destroyed = true;
        for (const off of offs) off();
      },
    };
  }),
);

export type NotificationsStoreType = InstanceType<typeof NotificationsStore>;
