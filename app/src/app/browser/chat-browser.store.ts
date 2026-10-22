import { inject } from '@angular/core';
import { patchState, signalStore, withHooks, withMethods, withState } from '@ngrx/signals';
import { ChatBrowserBackend } from './browser-backend';
import { AgentDecision, BrowserEvent, ChatBrowserMode, chatBrowserMode, ChatBrowserStatus, SourceInfo } from './browser.types';

interface ChatBrowserState {
  /** Per chat: read-along, agent mode, steps and allowed sites. */
  statuses: Record<string, ChatBrowserStatus>;
  /** Per chat: pages added for the next question. */
  sources: Record<string, SourceInfo[]>;
  error: string | null;
  busy: boolean;
}

const initial: ChatBrowserState = { statuses: {}, sources: {}, error: null, busy: false };

function message(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return $localize`:@@browser.chatStore.failed:The browser did not answer.`;
}

/**
 * A chat's hold on the browser as the chat panel and the Browser panel show it: the pages handed to it, read-
 * along, agent mode, its steps and Stop. The shell decides and enforces everything; this mirrors it.
 */
export const ChatBrowserStore = signalStore(
  { providedIn: 'root' },
  withState<ChatBrowserState>(initial),
  withMethods((store, backend = inject(ChatBrowserBackend)) => {
    const setStatus = (s: ChatBrowserStatus) => patchState(store, (st) => ({ statuses: { ...st.statuses, [s.sessionId]: s } }));
    const setSources = (id: string, list: SourceInfo[]) => patchState(store, (st) => ({ sources: { ...st.sources, [id]: list } }));
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
    const status = async (f: () => Promise<ChatBrowserStatus>) => {
      const s = await run(f);
      if (s) setStatus(s);
      return s;
    };
    return {
      onEvent(e: BrowserEvent): void {
        if (e.type === 'chatStatus') {
          const { type: _type, ...s } = e;
          setStatus(s);
        } else if (e.type === 'sources') {
          setSources(e.sessionId, e.sources);
        }
      },

      statusOf(id: string | null): ChatBrowserStatus | null {
        return id ? (store.statuses()[id] ?? null) : null;
      },

      sourcesOf(id: string | null): SourceInfo[] {
        return id ? (store.sources()[id] ?? []) : [];
      },

      /** Loads a chat's access and pages (when the chat is shown). */
      async load(id: string): Promise<void> {
        const [s, list] = await Promise.all([run(() => backend.status(id)), run(() => backend.sources(id))]);
        if (s) setStatus(s);
        if (list) setSources(id, list);
      },

      /** "Add page to AI context" for chat `id`. */
      async capture(id: string): Promise<boolean> {
        const list = await run(() => backend.capture(id));
        if (list) setSources(id, list);
        return list !== null;
      },

      async discardSources(id: string): Promise<void> {
        const list = await run(() => backend.discardSources(id));
        if (list) setSources(id, list);
      },

      /** The chat's next question took its pages along (the engine cleared them). */
      sent(id: string): void {
        setSources(id, []);
      },

      setReadAlong(id: string, on: boolean) {
        return status(() => backend.setReadAlong(id, on));
      },

      setAgent(id: string, on: boolean) {
        return status(() => backend.setAgent(id, on));
      },

      stop(id: string) {
        return status(() => backend.stop(id));
      },

      /**
       * Puts a chat's browser access in one mode: agent mode ends (denying waiting steps) before read-along changes,
       * and read-along is turned off for Off. Returns the status after the last change, null when one failed (the
       * error is shown; the status stays as the shell last reported it).
       */
      async setMode(id: string, mode: ChatBrowserMode): Promise<ChatBrowserStatus | null> {
        let s = store.statuses()[id] ?? null;
        if (chatBrowserMode(s) === mode) return s;
        if (mode === 'agent') return status(() => backend.setAgent(id, true));
        if (s?.agent) {
          s = await status(() => backend.setAgent(id, false));
          if (!s) return null;
        }
        const readAlong = mode === 'readAlong';
        if ((s?.readAlong ?? false) === readAlong) return s;
        return status(() => backend.setReadAlong(id, readAlong));
      },

      continueTask(id: string) {
        return status(() => backend.continueTask(id));
      },

      async respond(id: string, requestId: number, decision: AgentDecision): Promise<boolean> {
        return (await run(() => backend.respond(id, requestId, decision))) === true;
      },

      async openSuggested(id: string, url: string): Promise<boolean> {
        return (await run(() => backend.openSuggested(id, url))) !== null;
      },

      async openAndRead(id: string, url: string): Promise<boolean> {
        return (await run(() => backend.openAndRead(id, url))) !== null;
      },

      dismissError(): void {
        patchState(store, { error: null });
      },
    };
  }),
  withHooks((store, backend = inject(ChatBrowserBackend)) => {
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

export type ChatBrowserStoreType = InstanceType<typeof ChatBrowserStore>;
