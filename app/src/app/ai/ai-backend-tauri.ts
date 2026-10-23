import { invoke } from '../core/activity/ipc';
import { listenHere } from '../windowing/window-events';
import { AiBackend, toAiError, Unlisten } from './ai-backend';
import { cancelJob } from '../jobs/job-cancel';
import { jobValue } from '../jobs/job-value';
import {
  AI_CHAT_EVENT,
  AI_RUNTIME_EVENT,
  AI_SETUP_EVENT,
  AI_STATUS_EVENT,
  AI_UPDATES_EVENT,
  AiRuntimeEvent,
  AiRuntimeOverview,
  AiImage,
  AiStatus,
  AiUpdateKind,
  AiUpdatesStatus,
  ChatContext,
  ChatEnvelope,
  ManualDownload,
  ManualKind,
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
  DraftArgs,
  ResearchArgs,
  ResearchDraft,
  ResearchStarted,
} from './ai.types';

/** Subscribes to an event with its payload; tests pass fakes. */
export type ListenFn = (event: string, handler: (payload: unknown) => void) => Promise<Unlisten>;

const isObject = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === 'object' && !Array.isArray(v);

/** Talks to the Rust shell; `call` is Tauri's `invoke` and `listen` the window-scoped listener (tests pass fakes). */
export class TauriAiBackend extends AiBackend {
  /** The jobs of the set-up and runtime install this window started (Jobs are the one cancel authority). */
  private setupJob: number | null = null;
  private runtimeJob: number | null = null;

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
  refineFit() {
    return this.run<Recommendations>('ai_refine_fit');
  }
  catalog() {
    return this.run<Recommendation[]>('ai_catalog');
  }
  setupPlan(modelId?: string | null) {
    return this.run<SetupPlan>('ai_setup_plan', { modelId: modelId ?? null });
  }
  async setupRun(modelId: string) {
    this.setupJob = await this.run<number>('ai_setup_run', { modelId });
  }
  async setupBackground(modelId: string) {
    await this.run<number>('ai_setup_background', { modelId });
  }
  /** Cancels the set-up (or activation) this window started, through its job. */
  async setupCancel() {
    const id = this.setupJob;
    this.setupJob = null;
    try {
      return await cancelJob(this.call, id);
    } catch (e) {
      throw toAiError(e);
    }
  }
  models() {
    return this.run<InstalledModel[]>('ai_models');
  }
  async activateModel(fileName: string) {
    this.setupJob = await this.run<number>('ai_model_activate', { fileName });
  }
  deleteModel(fileName: string) {
    return this.run<void>('ai_model_delete', { fileName });
  }
  /** The command starts a download job and returns its id; the added model is the job's value. */
  async addLocalModel(path: string) {
    return this.modelJob(await this.run<number>('ai_model_add_local', { path }));
  }
  installProjector(fileName: string) {
    return this.run<number>('ai_model_install_projector', { fileName });
  }
  /** Like `addLocalModel`. */
  async addProjector(fileName: string, path: string) {
    return this.modelJob(await this.run<number>('ai_model_add_projector', { fileName, path }));
  }
  private async modelJob(job: number): Promise<InstalledModel> {
    try {
      return await jobValue<InstalledModel>(this.call, job);
    } catch (e) {
      throw toAiError(e);
    }
  }
  imageOpen(path: string) {
    return this.run<AiImage>('ai_image_open', { path });
  }
  imagePaste(data: string, name?: string | null) {
    return this.run<AiImage>('ai_image_paste', { data, name: name ?? null });
  }
  imageClose(id: number) {
    return this.run<boolean>('ai_image_close', { id });
  }
  reset() {
    return this.run<AiStatus>('ai_reset');
  }
  runtimes() {
    return this.run<AiRuntimeOverview>('ai_runtimes');
  }
  async runtimeInstall(id: string) {
    this.runtimeJob = await this.run<number>('ai_runtime_install', { id });
  }
  async runtimeAddArchives(id: string, paths: string[]) {
    this.runtimeJob = await this.run<number>('ai_runtime_add_archives', { id, paths });
  }
  updatesStatus() {
    return this.run<AiUpdatesStatus>('ai_updates_status');
  }
  updatesCheck(kind?: AiUpdateKind | null) {
    return this.run<AiUpdatesStatus>('ai_updates_check', { kind: kind ?? null });
  }
  updatesApply(key: string) {
    return this.run<AiUpdatesStatus>('ai_updates_apply', { key });
  }
  updatesSkip(key: string) {
    return this.run<AiUpdatesStatus>('ai_updates_skip', { key });
  }
  updatesLater(key: string) {
    return this.run<AiUpdatesStatus>('ai_updates_later', { key });
  }
  manualDownload(kind: ManualKind, id: string) {
    return this.run<ManualDownload>('ai_manual_download', { kind, id });
  }
  openManualDownload(kind: ManualKind, id: string, index: number) {
    return this.run<void>('ai_manual_download_open', { kind, id, index });
  }
  /** Cancels the runtime install this window started, through its job. */
  async runtimeCancel() {
    const id = this.runtimeJob;
    this.runtimeJob = null;
    try {
      return await cancelJob(this.call, id);
    } catch (e) {
      throw toAiError(e);
    }
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
  researchDraft(args: DraftArgs) {
    return this.run<ResearchDraft>('ai_research_draft', { args });
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
  chatSend(sessionId: string, text: string, context: ChatContext, images?: number[]) {
    return this.run<number>('ai_chat_send', { sessionId, text, context, images: images?.length ? images : null });
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
  onUpdates(handler: (s: AiUpdatesStatus) => void) {
    return this.listen(AI_UPDATES_EVENT, (p) => {
      if (isObject(p) && Array.isArray(p['kinds']) && typeof p['intervalHours'] === 'number')
        handler(p as unknown as AiUpdatesStatus);
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
