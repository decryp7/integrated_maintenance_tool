import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { TimeCursor } from '../../analysis/time-cursor';
import { WorkspaceStore } from '../../core/workspace.store';
import { GridSelection } from '../../grid/grid-selection';
import { AiBackend, toAiError } from '../ai-backend';
import { AiChatPersistence } from '../ai-chat-persistence';
import { ChatEventEnvelope, SessionInfo } from '../ai.types';
import { AgentApprovalState, ALL_CONTEXT, buildContext, ContextSources, ContextToggles, reduceTurn, startTurn, turnFromRecord, TurnView } from './chat-model';

interface ChatState {
  sessions: SessionInfo[];
  activeId: string | null;
  /** Turns per chat (loaded on first view, then kept up to date from the stream). */
  turns: Record<string, TurnView[]>;
  /** Last streamed event per chat (to resume after a reload). */
  lastSeq: Record<string, number>;
  prompts: string[];
  toggles: ContextToggles;
  error: string | null;
  notice: string | null;
  loading: boolean;
}

const initial: ChatState = { sessions: [], activeId: null, turns: {}, lastSeq: {}, prompts: [], toggles: ALL_CONTEXT, error: null, notice: null, loading: false };

/**
 * The chat panel's state for this window. The engine runs the turns and owns the history; this store
 * keeps what the panel shows, folding the `imt://ai-chat` stream into the running turn.
 */
export const ChatStore = signalStore(
  { providedIn: 'root' },
  withState<ChatState>(initial),
  withComputed((s, ws = inject(WorkspaceStore), cursor = inject(TimeCursor), grid = inject(GridSelection)) => ({
    active: computed(() => s.sessions().find((x) => x.id === s.activeId()) ?? null),
    activeTurns: computed(() => (s.activeId() ? (s.turns()[s.activeId()!] ?? []) : [])),
    running: computed(() => {
      const t = s.activeId() ? s.turns()[s.activeId()!] : undefined;
      return !!t?.at(-1)?.running;
    }),
    /** What the user is looking at, before the toggles. */
    sources: computed<ContextSources>(() => {
      const q = ws.query();
      const sel = cursor.selection();
      const range = sel && sel.to > sel.from ? sel : q.from != null && q.to != null && q.to > q.from ? { from: q.from, to: q.to } : null;
      const picked = grid.records().map((r) => r.id);
      const one = ws.selected()?.id;
      const selectedIds = picked.length ? picked : one != null ? [one] : [];
      return { ql: q.ql ?? null, range, selectedIds, instance: q.groups.length === 1 ? (q.groups[0] ?? null) : null, view: 'events grid' };
    }),
  })),
  withMethods((store, backend = inject(AiBackend), persistence = inject(AiChatPersistence)) => {
    const setTurns = (id: string, f: (t: TurnView[]) => TurnView[]) => patchState(store, (s) => ({ turns: { ...s.turns, [id]: f(s.turns[id] ?? []) } }));
    const fail = (e: unknown) => patchState(store, { error: toAiError(e).message });

    const refreshSessions = async () => {
      const sessions = await backend.chatSessions();
      patchState(store, { sessions });
      return sessions;
    };

    const loadTurns = async (id: string) => {
      const rec = await backend.chatGet(id);
      setTurns(id, (cur) => {
        const saved = rec.turns.map(turnFromRecord);
        // Keep a turn that still runs (it is not in the record yet).
        const live = cur.at(-1)?.running ? [cur.at(-1)!] : [];
        return [...saved, ...live];
      });
    };

    const methods = {
      /** Folds one streamed event into its chat. */
      onEvent(e: ChatEventEnvelope): void {
        const id = e.sessionId;
        if (e.seq > 0) {
          if (e.seq <= (store.lastSeq()[id] ?? 0)) return;
          patchState(store, (s) => ({ lastSeq: { ...s.lastSeq, [id]: e.seq } }));
        }
        setTurns(id, (turns) => {
          const last = turns.at(-1);
          if (!last?.running) return turns;
          return [...turns.slice(0, -1), reduceTurn(last, e)];
        });
        if (e.type === 'done' || e.type === 'error') {
          void refreshSessions().catch(() => undefined);
          void persistence.sync();
        }
      },

      async init(): Promise<void> {
        patchState(store, { loading: true, error: null });
        try {
          let sessions = await refreshSessions();
          if (!sessions.length) {
            await backend.chatNew();
            sessions = await refreshSessions();
          }
          const keep = sessions.some((x) => x.id === store.activeId()) ? store.activeId()! : sessions.at(-1)!.id;
          patchState(store, { activeId: keep });
          await loadTurns(keep);
          // Catch up with a turn that ran while the panel was closed.
          if (sessions.find((x) => x.id === keep)?.running) await methods.resume(keep);
          backend.suggestedPrompts().then(
            (prompts) => patchState(store, { prompts }),
            () => patchState(store, { prompts: [] }),
          );
        } catch (e) {
          fail(e);
        } finally {
          patchState(store, { loading: false });
        }
      },

      async resume(id: string): Promise<void> {
        const events = await backend.chatEventsSince(id, store.lastSeq()[id] ?? 0);
        if (events === null) {
          await loadTurns(id);
          return;
        }
        if (!store.turns()[id]?.at(-1)?.running) setTurns(id, (t) => [...t, startTurn('…')]);
        for (const e of events) methods.onEvent({ ...e, sessionId: id });
      },

      async select(id: string): Promise<void> {
        patchState(store, { activeId: id, error: null });
        if (!store.turns()[id]) await loadTurns(id).catch(fail);
      },

      async newChat(): Promise<void> {
        try {
          const s = await backend.chatNew();
          await refreshSessions();
          patchState(store, (st) => ({ activeId: s.id, turns: { ...st.turns, [s.id]: [] }, error: null }));
        } catch (e) {
          fail(e);
        }
      },

      async deleteChat(id: string): Promise<void> {
        try {
          await backend.chatDelete(id);
          const sessions = await refreshSessions();
          patchState(store, (s) => {
            const turns = { ...s.turns };
            delete turns[id];
            return { turns, activeId: s.activeId === id ? (sessions.at(-1)?.id ?? null) : s.activeId };
          });
          if (!sessions.length) await methods.newChat();
          else if (store.activeId() && !store.turns()[store.activeId()!]) await loadTurns(store.activeId()!);
          void persistence.sync();
        } catch (e) {
          fail(e);
        }
      },

      async send(text: string): Promise<boolean> {
        const id = store.activeId();
        const question = text.trim();
        if (!id || !question || store.running()) return false;
        patchState(store, { error: null, notice: null });
        setTurns(id, (t) => [...t, startTurn(question)]);
        try {
          await backend.chatSend(id, question, buildContext(store.sources(), store.toggles()));
          return true;
        } catch (e) {
          setTurns(id, (t) => (t.at(-1)?.running ? t.slice(0, -1) : t));
          fail(e);
          return false;
        }
      },

      async cancel(): Promise<void> {
        const id = store.activeId();
        if (id) await backend.chatCancel(id).catch(fail);
      },

      /** Shows the user's answer on an agent approval card of the running turn. */
      setApprovalState(requestId: number, state: AgentApprovalState): void {
        const id = store.activeId();
        if (!id) return;
        setTurns(id, (turns) =>
          turns.map((t) => (t.running ? { ...t, approvals: t.approvals.map((a) => (a.request.requestId === requestId ? { ...a, state } : a)) } : t)),
        );
      },

      setProposalState(turnIndex: number, proposalIndex: number, state: 'applied' | 'dismissed'): void {
        const id = store.activeId();
        if (!id) return;
        setTurns(id, (turns) =>
          turns.map((t, i) => (i === turnIndex ? { ...t, proposals: t.proposals.map((p, j) => (j === proposalIndex ? { ...p, state } : p)) } : t)),
        );
      },

      toggle(key: keyof ContextToggles): void {
        patchState(store, (s) => ({ toggles: { ...s.toggles, [key]: !s.toggles[key] } }));
      },

      setNotice(notice: string | null): void {
        patchState(store, { notice });
      },

      dismissError(): void {
        patchState(store, { error: null });
      },

      /** Saved chats were restored (project opened): start over from the engine's chats. */
      async reload(): Promise<void> {
        patchState(store, { turns: {}, lastSeq: {}, activeId: null });
        await methods.init();
      },
    };
    return methods;
  }),
  withHooks((store, backend = inject(AiBackend)) => {
    let off: (() => void) | undefined;
    let destroyed = false;
    return {
      onInit() {
        void backend
          .onChat((e) => store.onEvent(e))
          .then((u) => (destroyed ? u() : (off = u)))
          .catch(() => undefined);
      },
      // Stop listening when the store goes away (the window's injector is destroyed).
      onDestroy() {
        destroyed = true;
        off?.();
      },
    };
  }),
);

export type ChatStoreType = InstanceType<typeof ChatStore>;
