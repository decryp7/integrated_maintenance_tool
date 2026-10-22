import { computed, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withHooks, withMethods, withState } from '@ngrx/signals';
import { TimeCursor } from '../../analysis/time-cursor';
import { WorkspaceStore } from '../../core/workspace.store';
import { GridSelection } from '../../grid/grid-selection';
import { AiBackend, toAiError } from '../ai-backend';
import { AiChatPersistence } from '../ai-chat-persistence';
import { AiImage, ChatEventEnvelope, SessionInfo } from '../ai.types';
import { AiStatusStore } from '../ai-status.store';
import { AiImageStore } from '../vision/ai-image.store';
import { MAX_IMAGES_PER_QUESTION } from '../vision/image-files';
import { FrameThrottle } from '../../shared/throttle';
import { AgentApprovalState, buildContext, ContextMode, ContextSources, contextToggles, effectiveContextMode, reduceTurn, startTurn, turnFromRecord, TurnView } from './chat-model';

interface ChatState {
  sessions: SessionInfo[];
  activeId: string | null;
  /** Turns per chat (loaded on first view, then kept up to date from the stream). */
  turns: Record<string, TurnView[]>;
  /** Last streamed event per chat (to resume after a reload). */
  lastSeq: Record<string, number>;
  prompts: string[];
  /** The context the user chose in the composer; null: automatic (the selected events if any, else the view). */
  contextChoice: ContextMode | null;
  error: string | null;
  notice: string | null;
  loading: boolean;
  /** Images attached to the next question (open images; at most 4). */
  attachments: AiImage[];
  /** What each turn of this session showed the model, by chat and turn index (the engine's history says "[image]"). */
  turnImages: Record<string, Record<number, ImageThumb[]>>;
}

/** A small picture of an attached image: its clean copy only. */
export type ImageThumb = Pick<AiImage, 'id' | 'name' | 'dataUrl'>;

const initial: ChatState = { sessions: [], activeId: null, turns: {}, lastSeq: {}, prompts: [], contextChoice: null, error: null, notice: null, loading: false, attachments: [], turnImages: {} };

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
  withComputed((s) => ({
    /** The context sent with the next question (the composer's "Context" menu). */
    contextMode: computed(() => effectiveContextMode(s.contextChoice(), s.sources())),
  })),
  withComputed((s) => ({
    toggles: computed(() => contextToggles(s.contextMode())),
  })),
  withMethods((store, backend = inject(AiBackend), persistence = inject(AiChatPersistence), status = inject(AiStatusStore), images = inject(AiImageStore)) => {
    const setTurns = (id: string, f: (t: TurnView[]) => TurnView[]) => patchState(store, (s) => ({ turns: { ...s.turns, [id]: f(s.turns[id] ?? []) } }));
    const fail = (e: unknown) => patchState(store, { error: toAiError(e).message });

    // Streamed text arrives a few characters at a time: fold it into the turn once per frame, not once per token (each
    // fold re-parses and re-renders the answer). Any other event first applies the text before it, keeping the order.
    const pendingText = new Map<string, string>();
    const textFrame = new FrameThrottle();
    const flushText = () => {
      textFrame.cancel();
      for (const [id, text] of pendingText) {
        setTurns(id, (turns) => {
          const last = turns.at(-1);
          return last?.running ? [...turns.slice(0, -1), reduceTurn(last, { type: 'delta', text })] : turns;
        });
      }
      pendingText.clear();
    };

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
        if (e.type === 'delta') {
          pendingText.set(id, (pendingText.get(id) ?? '') + e.text);
          textFrame.run(flushText);
          return;
        }
        flushText();
        // An answer recorded outside a turn (a research from a Research button): read the chat again.
        // (Not after a turn that ended with an error the user still sees: its own late `done` must not hide it.)
        const lastTurn = store.turns()[id]?.at(-1);
        if (e.type === 'done' && !lastTurn?.running && !lastTurn?.error && !lastTurn?.cancelled) void loadTurns(id).catch(() => undefined);
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
        flushText();
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

      /**
       * Sends a question with the attached images (or `with`, e.g. one chip's "Find related logs"); the images sent are
       * detached. A text-only model is not asked about images: the no-vision message says what to do instead.
       */
      async send(text: string, withImages?: AiImage[]): Promise<boolean> {
        const id = store.activeId();
        const question = text.trim();
        if (!id || !question || store.running()) return false;
        const shown = withImages ?? store.attachments();
        if (shown.length && status.status()?.vision === false) {
          images.refuseNoVision();
          return false;
        }
        patchState(store, { error: null, notice: null });
        const index = store.turns()[id]?.length ?? 0;
        setTurns(id, (t) => [...t, startTurn(question)]);
        try {
          await backend.chatSend(id, question, buildContext(store.sources(), store.toggles()), shown.length ? shown.map((i) => i.id) : undefined);
          if (shown.length) {
            const sent = new Set(shown.map((i) => i.id));
            const thumbs = shown.map(({ id: imageId, name, dataUrl }) => ({ id: imageId, name, dataUrl }));
            patchState(store, (s) => ({
              attachments: s.attachments.filter((a) => !sent.has(a.id)),
              turnImages: { ...s.turnImages, [id]: { ...s.turnImages[id], [index]: thumbs } },
            }));
          }
          return true;
        } catch (e) {
          setTurns(id, (t) => (t.at(-1)?.running ? t.slice(0, -1) : t));
          if (toAiError(e).code === 'noVision') images.refuseNoVision();
          else fail(e);
          return false;
        }
      },

      /** Attaches an open image to the next question (at most 4; the same image once). */
      attach(image: AiImage): boolean {
        if (store.attachments().some((a) => a.id === image.id)) return true;
        if (store.attachments().length >= MAX_IMAGES_PER_QUESTION) {
          patchState(store, { notice: $localize`:@@ai.vision.chat.tooMany:At most ${MAX_IMAGES_PER_QUESTION}:max: images go with one question.` });
          return false;
        }
        patchState(store, (s) => ({ attachments: [...s.attachments, image] }));
        return true;
      },

      detach(imageId: number): void {
        patchState(store, (s) => ({ attachments: s.attachments.filter((a) => a.id !== imageId) }));
      },

      /** "Find related logs" on an attachment chip: asks for the logs related to that image. */
      sendRelated(image: AiImage): Promise<boolean> {
        return methods.send($localize`:@@ai.vision.chat.relatedQuestion:find the logs related to this screenshot`, [image]);
      },

      /** The pictures attached to turn `index` of the active chat in this session. */
      imagesOf(index: number): ImageThumb[] {
        const id = store.activeId();
        return id ? (store.turnImages()[id]?.[index] ?? []) : [];
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

      /** Chooses what goes with the next questions (null: automatic again). */
      setContextMode(mode: ContextMode | null): void {
        patchState(store, { contextChoice: mode });
      },

      setNotice(notice: string | null): void {
        patchState(store, { notice });
      },

      dismissError(): void {
        patchState(store, { error: null });
      },

      setError(error: string | null): void {
        patchState(store, { error });
      },

      /** Saved chats were restored (project opened): start over from the engine's chats. */
      async reload(): Promise<void> {
        patchState(store, { turns: {}, lastSeq: {}, activeId: null, turnImages: {} });
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
