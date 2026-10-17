import { inject, Injectable, Provider } from '@angular/core';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import {
  AiErrorCode,
  AiStatus,
  ApprovalDecision,
  ChatContext,
  ChatEnvelope,
  ChatEventEnvelope,
  ChatHistoryRecord,
  ChatRecord,
  ExtractorsProposal,
  Hardware,
  InstalledModel,
  MarkersProposal,
  Narrative,
  PipelineProposal,
  ProfileProposal,
  QueryProposal,
  Recommendation,
  Recommendations,
  RegexProposal,
  RestoreResult,
  SampleSource,
  SessionInfo,
  SetupEvent,
  SetupPlan,
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
  if (e && typeof e === 'object' && typeof (e as { code?: unknown }).code === 'string' && typeof (e as { message?: unknown }).message === 'string') {
    return new AiError((e as { code: AiErrorCode }).code, (e as { message: string }).message);
  }
  if (e instanceof Error) return new AiError('engine', e.message);
  return new AiError('engine', typeof e === 'string' ? e : 'The AI assistant failed.');
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
  abstract catalog(): Promise<Recommendation[]>;
  abstract setupPlan(modelId?: string | null): Promise<SetupPlan>;
  /** Starts a set-up; progress arrives through `onSetup`. */
  abstract setupRun(modelId: string): Promise<void>;
  abstract setupCancel(): Promise<boolean>;
  abstract models(): Promise<InstalledModel[]>;
  /** Loads an installed model and makes it active; progress arrives through `onSetup`. */
  abstract activateModel(fileName: string): Promise<void>;
  abstract deleteModel(fileName: string): Promise<void>;
  abstract addLocalModel(path: string): Promise<InstalledModel>;
  abstract reset(): Promise<AiStatus>;

  abstract proposeRegex(lines: string[], intent?: string | null): Promise<RegexProposal>;
  abstract proposeProfile(source: SampleSource): Promise<ProfileProposal>;
  abstract proposePipeline(goal: string, source: SampleSource): Promise<PipelineProposal>;
  abstract proposeMarkers(goal: string, source: SampleSource): Promise<MarkersProposal>;
  abstract proposeExtractors(goal: string, source: SampleSource): Promise<ExtractorsProposal>;
  abstract nlToQuery(question: string): Promise<QueryProposal>;
  abstract explainEvent(recordId: number): Promise<Narrative>;
  abstract summarize(from: number, to: number): Promise<Narrative>;
  abstract rootCauses(): Promise<Narrative>;
  /** Stops this window's running use cases. */
  abstract cancel(): Promise<number>;

  abstract chatSessions(): Promise<SessionInfo[]>;
  abstract chatNew(): Promise<SessionInfo>;
  abstract chatGet(sessionId: string): Promise<ChatRecord>;
  abstract chatDelete(sessionId: string): Promise<boolean>;
  /** Starts a turn; the answer streams through `onChat`. */
  abstract chatSend(sessionId: string, text: string, context: ChatContext): Promise<void>;
  abstract chatCancel(sessionId: string): Promise<boolean>;
  abstract chatEventsSince(sessionId: string, seq: number): Promise<ChatEnvelope[] | null>;
  abstract chatHistory(): Promise<ChatHistoryRecord>;
  abstract chatRestore(record: unknown): Promise<RestoreResult>;
  /** Writes the chat as Markdown; refuses to replace an existing file unless `overwrite`. */
  abstract chatExportMarkdown(sessionId: string, path: string, overwrite?: boolean): Promise<string>;
  abstract suggestedPrompts(): Promise<string[]>;
  abstract approvalRespond(sessionId: string, requestId: number, decision: ApprovalDecision): Promise<boolean>;
  /** Opens a web source an answer showed in the system browser (anything else is refused by the engine). */
  abstract openLink(url: string): Promise<void>;

  abstract setBraveKey(key: string): Promise<void>;
  abstract clearBraveKey(): Promise<void>;
  abstract braveKeySet(): Promise<boolean>;

  abstract onSetup(handler: (e: SetupEvent) => void): Promise<Unlisten>;
  abstract onStatus(handler: (s: AiStatus) => void): Promise<Unlisten>;
  abstract onChat(handler: (e: ChatEventEnvelope) => void): Promise<Unlisten>;
}

/** Loads the backend for this host: the Tauri one inside the app, the scripted mock in a plain browser. */
export function loadAiBackend(): Promise<AiBackend> {
  if (isTauri()) return import('./ai-backend-tauri').then((m) => new m.TauriAiBackend());
  return import('../core/mock/mock-ai-backend').then((m) => new m.MockAiBackend(m.mockAiOptionsFromSearch(typeof window === 'undefined' ? '' : window.location.search)));
}

/** The AI backend, loaded on first use so neither adapter weighs on the initial bundle. */
export function provideAiBackend(): Provider[] {
  // Behind the shared backend proxy like every backend: failures are logged and calls join the activity log.
  return [{ provide: AiBackend, useFactory: () => withFailureLogging<AiBackend>(lazyAiBackend(loadAiBackend), inject(Logger), { category: 'ui.ai' }) }];
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
          const b = (await real) as unknown as Record<PropertyKey, (...a: unknown[]) => Promise<unknown>>;
          return b[name](...args);
        };
        methods.set(name, m);
      }
      return m;
    },
  });
}
