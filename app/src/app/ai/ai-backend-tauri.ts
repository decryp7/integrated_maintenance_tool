import { invoke } from '../core/activity/ipc';
import { listenHere } from '../windowing/window-events';
import { AiBackend, toAiError, Unlisten } from './ai-backend';
import {
  AI_CHAT_EVENT,
  AI_RUNTIME_EVENT,
  AI_SETUP_EVENT,
  AI_STATUS_EVENT,
  AiRuntimeEvent,
  AiRuntimeOverview,
  AiStatus,
  ChatContext,
  ChatEnvelope,
  ChatEventEnvelope,
  ChatHistoryRecord,
  ChatRecord,
  Hardware,
  InstalledModel,
  AiStartRequest,
  Recommendation,
  Recommendations,
  RestoreResult,
  SessionInfo,
  SetupEvent,
  SetupPlan,
  ResearchArgs,
  ResearchStarted,
} from './ai.types';

/** Subscribes to an event with its payload; tests pass fakes. */
export type ListenFn = (event: string, handler: (payload: unknown) => void) => Promise<Unlisten>;

const isObject = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === 'object' && !Array.isArray(v);

/** Talks to the Rust shell; `call` is Tauri's `invoke` and `listen` the window-scoped listener (tests pass fakes). */
export class TauriAiBackend extends AiBackend {
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly listen: ListenFn = listenHere,
  ) {
    super();
  }

  private async run<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    try {
      return await this.call<T>(cmd, args);
    } catch (e) {
      throw toAiError(e);
    }
  }

  status() {
    return this.run<AiStatus>('ai_status');
  }
  hardware() {
    return this.run<Hardware>('ai_hardware');
  }
  recommend() {
    return this.run<Recommendations>('ai_recommend');
  }
  catalog() {
    return this.run<Recommendation[]>('ai_catalog');
  }
  setupPlan(modelId?: string | null) {
    return this.run<SetupPlan>('ai_setup_plan', { modelId: modelId ?? null });
  }
  setupRun(modelId: string) {
    return this.run<void>('ai_setup_run', { modelId });
  }
  setupCancel() {
    return this.run<boolean>('ai_setup_cancel');
  }
  models() {
    return this.run<InstalledModel[]>('ai_models');
  }
  activateModel(fileName: string) {
    return this.run<void>('ai_model_activate', { fileName });
  }
  deleteModel(fileName: string) {
    return this.run<void>('ai_model_delete', { fileName });
  }
  addLocalModel(path: string) {
    return this.run<InstalledModel>('ai_model_add_local', { path });
  }
  reset() {
    return this.run<AiStatus>('ai_reset');
  }
  runtimes() {
    return this.run<AiRuntimeOverview>('ai_runtimes');
  }
  runtimeInstall(id: string) {
    return this.run<void>('ai_runtime_install', { id });
  }
  runtimeCancel() {
    return this.run<boolean>('ai_runtime_cancel');
  }
  runtimeDelete(tag: string, id: string) {
    return this.run<void>('ai_runtime_delete', { tag, id });
  }
  runtimeChoose(id: string) {
    return this.run<void>('ai_runtime_choose', { id });
  }
  start(request: AiStartRequest) {
    return this.run<number>('ai_start', { request });
  }
  research(args: ResearchArgs) {
    return this.run<ResearchStarted>('ai_research', { args });
  }
  cancel() {
    return this.run<number>('ai_cancel');
  }
  chatSessions() {
    return this.run<SessionInfo[]>('ai_chat_sessions');
  }
  chatNew() {
    return this.run<SessionInfo>('ai_chat_new');
  }
  chatGet(sessionId: string) {
    return this.run<ChatRecord>('ai_chat_get', { sessionId });
  }
  chatDelete(sessionId: string) {
    return this.run<boolean>('ai_chat_delete', { sessionId });
  }
  chatSend(sessionId: string, text: string, context: ChatContext) {
    return this.run<number>('ai_chat_send', { sessionId, text, context });
  }
  chatCancel(sessionId: string) {
    return this.run<boolean>('ai_chat_cancel', { sessionId });
  }
  chatEventsSince(sessionId: string, seq: number) {
    return this.run<ChatEnvelope[] | null>('ai_chat_events_since', { sessionId, seq });
  }
  chatHistory() {
    return this.run<ChatHistoryRecord>('ai_chat_history');
  }
  chatRestore(record: unknown) {
    return this.run<RestoreResult>('ai_chat_restore', { record: record ?? null });
  }
  chatExportMarkdown(sessionId: string, path: string, overwrite = false) {
    return this.run<string>('ai_chat_export_markdown', {
      sessionId,
      path,
      ...(overwrite ? { overwrite } : {}),
    });
  }
  suggestedPrompts() {
    return this.run<string[]>('ai_suggested_prompts');
  }
  openLink(url: string) {
    return this.run<void>('ai_open_link', { url });
  }
  onSetup(handler: (e: SetupEvent) => void) {
    return this.listen(AI_SETUP_EVENT, (p) => {
      if (isObject(p) && typeof p['stage'] === 'string') handler(p as unknown as SetupEvent);
    });
  }
  onRuntime(handler: (e: AiRuntimeEvent) => void) {
    return this.listen(AI_RUNTIME_EVENT, (p) => {
      if (isObject(p) && typeof p['stage'] === 'string' && typeof p['id'] === 'string')
        handler(p as unknown as AiRuntimeEvent);
    });
  }
  onStatus(handler: (s: AiStatus) => void) {
    return this.listen(AI_STATUS_EVENT, (p) => {
      if (isObject(p) && typeof p['state'] === 'string' && typeof p['available'] === 'boolean')
        handler(p as unknown as AiStatus);
    });
  }
  onChat(handler: (e: ChatEventEnvelope) => void) {
    return this.listen(AI_CHAT_EVENT, (p) => {
      if (
        isObject(p) &&
        typeof p['sessionId'] === 'string' &&
        typeof p['type'] === 'string' &&
        typeof p['seq'] === 'number'
      )
        handler(p as unknown as ChatEventEnvelope);
    });
  }
}
