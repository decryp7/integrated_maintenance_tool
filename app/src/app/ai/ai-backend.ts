import { inject, Injectable, Provider } from '@angular/core';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import {
  AiErrorCode,
  AiRuntimeEvent,
  AiRuntimeOverview,
  AiStatus,
  PitwallStatus,
  AiUpdateKind,
  AiUpdatesStatus,
  ChatContext,
  ManualDownload,
  ManualKind,
  ChatEnvelope,
  ChatEventEnvelope,
  ChatHistoryRecord,
  ChatRecord,
  Hardware,
  InstalledModel,
  Recommendation,
  Recommendations,
  RestoreResult,
  SessionInfo,
  SetupEvent,
  SetupPlan,
  AiStartRequest,
  AiImage,
  DraftArgs,
  ResearchArgs,
  ResearchDraft,
  ResearchStarted,
} from './ai.types';

/** A rejected AI command: a stable `code` (hide AI, offer set-up, retry) and a message for the user. */
export class AiError extends Error {
  constructor(
    readonly code: AiErrorCode,
    message: string,
  ) {
    super(message);
    this.name = 'AiError';
  }
}

/** Turns whatever a command rejected with into an `AiError` (never trusts the wire blindly). */
export function toAiError(e: unknown): AiError {
  if (e instanceof AiError) return e;
  if (
    e &&
    typeof e === 'object' &&
    typeof (e as { code?: unknown }).code === 'string' &&
    typeof (e as { message?: unknown }).message === 'string'
  ) {
    return new AiError((e as { code: AiErrorCode }).code, (e as { message: string }).message);
  }
  if (e instanceof Error) return new AiError('engine', e.message);
  return new AiError('engine', typeof e === 'string' ? e : $localize`:@@ai.backend.failed:The AI assistant failed.`);
}

export type Unlisten = () => void;

/** Everything the UI asks of the local AI assistant. Injected as a class token (Tauri or the mock). */
// Defaults to the lazily loaded adapter for this host, so views with AI entry points work without
// extra providers (e.g. in other features' tests); `provideAiBackend` / tests can still override it.
@Injectable({ providedIn: 'root', useFactory: () => lazyAiBackend(loadAiBackend) })
export abstract class AiBackend {
  abstract status(): Promise<AiStatus>;
  abstract hardware(): Promise<Hardware>;
  abstract recommend(): Promise<Recommendations>;
  /** As `recommend`, after the engine has read the model headers (slow, network): exact context sizes. */
  abstract refineFit(): Promise<Recommendations>;
  abstract catalog(): Promise<Recommendation[]>;
  abstract setupPlan(modelId?: string | null): Promise<SetupPlan>;
  /** Starts a set-up; progress arrives through `onSetup`. */
  abstract setupRun(modelId: string): Promise<void>;
  /**
   * The first-start set-up as a background job: the inference runtime when none is installed, then the model. Resolves
   * once it is queued; progress is in the Jobs pane, the status bar and `onSetup`, and a message says how it ended.
   */
  abstract setupBackground(modelId: string): Promise<void>;
  abstract setupCancel(): Promise<boolean>;
  abstract models(): Promise<InstalledModel[]>;
  /** Loads an installed model and makes it active; progress arrives through `onSetup`. */
  abstract activateModel(fileName: string): Promise<void>;
  abstract deleteModel(fileName: string): Promise<void>;
  abstract addLocalModel(path: string): Promise<InstalledModel>;
  /** "Add image support": downloads the catalog projector of an installed model as a download job (resolves with its
   * id). Never called without the user asking. */
  abstract installProjector(fileName: string): Promise<number>;
  /** Adds a projector file the user chose for an installed model (checked like a download). */
  abstract addProjector(fileName: string, path: string): Promise<InstalledModel>;
  abstract reset(): Promise<AiStatus>;

  /** Starts pairing with Pitwall (or, when already paired, just chooses it when `useForDebrief`); returns at once with
   * the code to compare while the pairing waits for approval. Changes also arrive through `onStatus`. */
  abstract pitwallConnect(useForDebrief: boolean): Promise<PitwallStatus>;
  /** Stops a pairing that waits for approval. */
  abstract pitwallCancel(): Promise<boolean>;
  /** Deletes the stored Pitwall key. */
  abstract pitwallForget(): Promise<AiStatus>;
  /** Probes Pitwall now. */
  abstract pitwallRefresh(): Promise<AiStatus>;
  /** Switches this session to the built-in engine. */
  abstract useBuiltInEngine(): Promise<AiStatus>;

  /** The inference runtimes (llama-server builds) for this platform: installed, usable, suggested, active. */
  abstract runtimes(): Promise<AiRuntimeOverview>;
  /** Starts downloading a build; progress arrives through `onRuntime`. */
  abstract runtimeInstall(id: string): Promise<void>;
  abstract runtimeCancel(): Promise<boolean>;
  abstract runtimeDelete(tag: string, id: string): Promise<void>;
  /** Makes an installed build (or `auto`) the one that runs the model; the engine restarts with it. */
  abstract runtimeChoose(id: string): Promise<void>;
  /** Installs a build from its archives downloaded by hand (checked against the pinned SHA-256); progress
   * arrives through `onRuntime` like a download. */
  abstract runtimeAddArchives(id: string, paths: string[]): Promise<void>;

  /** The runtime and model update checks: policies, last check, what is available. */
  abstract updatesStatus(): Promise<AiUpdatesStatus>;
  /** "Check now" for one kind, or both; resolves when the check is done. */
  abstract updatesCheck(kind?: AiUpdateKind | null): Promise<AiUpdatesStatus>;
  /** "Update": installs an update now, as a download job. */
  abstract updatesApply(key: string): Promise<AiUpdatesStatus>;
  /** "Skip this version". */
  abstract updatesSkip(key: string): Promise<AiUpdatesStatus>;
  /** "Later": asked again at the next check. */
  abstract updatesLater(key: string): Promise<AiUpdatesStatus>;

  /** The files to download by hand for a model or runtime build ("Download manually"). */
  abstract manualDownload(kind: ManualKind, id: string): Promise<ManualDownload>;
  /** Opens file `index` of that download in the system browser (the engine computes the address). */
  abstract openManualDownload(kind: ManualKind, id: string, index: number): Promise<void>;

  /** Starts a use case (explain, a proposal, a query...) as a background AI job and resolves with its id at once.
   * Its answer is the job's result: a dialog that is open follows it (`JobFollow`), otherwise its message ("Explanation
   * ready · View") opens it. Rejects at once when it cannot start (no model, invalid arguments, a full queue). */
  abstract start(request: AiStartRequest): Promise<number>;
  /** Web research ("Research this", the chat's "Research…"): a research job whose answer becomes a chat turn, or a
   * search text with workspace data to confirm first. */
  abstract research(args: ResearchArgs): Promise<ResearchStarted>;
  /** "Research this": the model drafts the search text from the question and the event (the engine's text without a
   * model); the leak guard checks it. Nothing is searched: the user sees and edits it first. */
  abstract researchDraft(args: DraftArgs): Promise<ResearchDraft>;
  /** Stops this window's running use cases. */
  abstract cancel(): Promise<number>;

  abstract chatSessions(): Promise<SessionInfo[]>;
  abstract chatNew(): Promise<SessionInfo>;
  abstract chatGet(sessionId: string): Promise<ChatRecord>;
  abstract chatDelete(sessionId: string): Promise<boolean>;
  /** Starts a turn as an AI job (resolves with its id); the answer streams through `onChat` once it runs. `images`
   * are open images (`imageOpen`, `imagePaste`) shown to the model with the question (ADR 0025). */
  abstract chatSend(sessionId: string, text: string, context: ChatContext, images?: number[]): Promise<number>;

  /** Opens an image file the user chose or the Explorer lists: Rust decodes it and returns the clean copy. */
  abstract imageOpen(path: string): Promise<AiImage>;
  /** Opens pasted or dropped image bytes (base64; the webview does not decode them). */
  abstract imagePaste(data: string, name?: string | null): Promise<AiImage>;
  /** Forgets an open image. */
  abstract imageClose(id: number): Promise<boolean>;
  abstract chatCancel(sessionId: string): Promise<boolean>;
  abstract chatEventsSince(sessionId: string, seq: number): Promise<ChatEnvelope[] | null>;
  abstract chatHistory(): Promise<ChatHistoryRecord>;
  abstract chatRestore(record: unknown): Promise<RestoreResult>;
  /** Writes the chat as Markdown; refuses to replace an existing file unless `overwrite`. */
  abstract chatExportMarkdown(
    sessionId: string,
    path: string,
    overwrite?: boolean,
  ): Promise<string>;
  abstract suggestedPrompts(): Promise<string[]>;
  /** Opens a web source an answer showed in the system browser (anything else is refused by the engine). */
  abstract openLink(url: string): Promise<void>;

  abstract onSetup(handler: (e: SetupEvent) => void): Promise<Unlisten>;
  abstract onRuntime(handler: (e: AiRuntimeEvent) => void): Promise<Unlisten>;
  abstract onStatus(handler: (s: AiStatus) => void): Promise<Unlisten>;
  abstract onUpdates(handler: (s: AiUpdatesStatus) => void): Promise<Unlisten>;
  abstract onChat(handler: (e: ChatEventEnvelope) => void): Promise<Unlisten>;
}

/** Loads the backend for this host: the Tauri one inside the app, the scripted mock in a plain browser. */
export function loadAiBackend(): Promise<AiBackend> {
  if (isTauri()) return import('./ai-backend-tauri').then((m) => new m.TauriAiBackend());
  return import('../core/mock/mock-ai-backend').then(
    (m) =>
      new m.MockAiBackend(
        m.mockAiOptionsFromSearch(typeof window === 'undefined' ? '' : window.location.search),
      ),
  );
}

/** The AI backend, loaded on first use so neither adapter weighs on the initial bundle. */
export function provideAiBackend(): Provider[] {
  // Behind the shared backend proxy like every backend: failures are logged and calls join the activity log.
  return [
    {
      provide: AiBackend,
      useFactory: () =>
        withFailureLogging<AiBackend>(lazyAiBackend(loadAiBackend), inject(Logger), {
          category: 'ui.ai',
        }),
    },
  ];
}

/** An `AiBackend` whose every method waits for the real one, loaded once on the first call. */
export function lazyAiBackend(load: () => Promise<AiBackend>): AiBackend {
  let real: Promise<AiBackend> | undefined;
  const methods = new Map<PropertyKey, (...args: unknown[]) => Promise<unknown>>();
  return new Proxy({} as AiBackend, {
    get(_target, name) {
      // Not a thenable, and no Angular lifecycle hooks (the injector would call `ngOnDestroy` on teardown).
      if (typeof name !== 'string' || name === 'then' || name.startsWith('ng')) return undefined;
      let m = methods.get(name);
      if (!m) {
        m = async (...args: unknown[]) => {
          real ??= load();
          const b = (await real) as unknown as Record<
            PropertyKey,
            (...a: unknown[]) => Promise<unknown>
          >;
          const method = b[name];
          if (!method) throw new TypeError(`${name} is not a function`);
          return method.apply(b, args);
        };
        methods.set(name, m);
      }
      return m;
    },
  });
}
