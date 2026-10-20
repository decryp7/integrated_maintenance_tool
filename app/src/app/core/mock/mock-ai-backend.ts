/**
 * A scripted local-AI backend for the browser build (ng serve, Playwright) and UI tests. It mimics the
 * contract of `app/src-tauri/src/ai` with plausible data: set-up progress on an 8 GB VRAM laptop, streamed
 * chat answers with tool activity, proposals and citations. Never the reference for
 * behaviour: the engine is.
 */
import { AiBackend, AiError, Unlisten } from '../../ai/ai-backend';
import {
  AiErrorCode,
  AiRuntimeEvent,
  AiStatus,
  AiUpdateKind,
  AiUpdatesStatus,
  ChatContext,
  ChatEnvelope,
  ChatEvent,
  ChatEventEnvelope,
  ChatHistoryRecord,
  ChatRecord,
  Citation,
  InstalledModel,
  ManualDownload,
  ManualKind,
  Narrative,
  Proposal,
  QueryProposal,
  RestoreResult,
  SampleSource,
  SessionInfo,
  SetupEvent,
  TurnRecord,
} from '../../ai/ai.types';
import {
  demoRecordIds,
  installedFrom,
  MOCK_CATALOG,
  catalogModel,
  MOCK_HARDWARE,
  mockExtractorsProposal,
  mockMarkersProposal,
  mockPipelineProposal,
  mockProfileProposal,
  mockRecommendations,
  mockRegexProposal,
} from './mock-ai-data';
import { FILE_EXISTS_SUFFIX } from '../api.types';
import { MOCK_NET_FAILURES, MockNetFailure, mockNetError, MockRuntimeMode, MockRuntimes } from './mock-ai-runtimes';
import { mockWeb } from '../../browser/mock-browser-backend';
import { MockAiUpdates, MockUpdatesMode } from './mock-ai-updates';
import { mockBrowserTurn } from './mock-ai-browser';
import { MockJobCancelled, mockJobs, mockSteps } from '../../jobs/mock-jobs';
import { JobLink, JobResult } from '../../jobs/jobs.types';
import { mockLeaks, runMockResearch, startMockResearch, wantsResearch, webCitations } from './mock-ai-research';
import { AiStartRequest, ResearchArgs, ResearchStarted } from '../../ai/ai.types';

/** Operations `mockAiFail` may name: their calls reject with an engine error. */
export const AI_FAILABLE = [
  'setup',
  'chat',
  'nlToQuery',
  'explain',
  'summarize',
  'rootCauses',
  'propose',
  'models',
  'runtime',
] as const;
export type AiFailable = (typeof AI_FAILABLE)[number];

export interface MockAiOptions {
  /** `ready` (a model is active), `setup` (nothing set up yet), `off` (no AI in this build/run). */
  mode?: 'ready' | 'setup' | 'off';
  /** Delay between streamed pieces, ms (0 in unit tests). */
  stepMs?: number;
  fail?: AiFailable[];
  /** The inference runtimes on disk (default `installed`: Vulkan works, CUDA is suggested). */
  runtime?: MockRuntimeMode;
  /** Model and runtime downloads fail like on a corporate network (e.g. `tls`: a TLS-inspecting proxy). */
  net?: MockNetFailure;
  /** Runtime and model update checks: `available` (an update with the Ask policy), `automatic`, `manual`. */
  updates?: MockUpdatesMode;
}

/**
 * Browser-demo switches (never used inside Tauri):
 *   ?mockAi=setup|off        start without a model, or with AI unavailable (hidden)
 *   ?mockAiFail=chat,setup   these operations fail (`runtime`: runtime downloads)
 *   ?mockAiRuntime=none|broken  no inference runtime installed, or one that finds no GPU
 *   ?mockAiNet=tls|proxyAuth|proxy|offline|timeout|httpStatus  downloads fail like behind a corporate proxy
 *   ?mockAiUpdates=available|automatic|manual  a runtime update to find (Ask, or that policy)
 *   ?mockAiStepMs=5          streaming speed
 */
export function mockAiOptionsFromSearch(search: string): MockAiOptions {
  const p = new URLSearchParams(search);
  const o: MockAiOptions = {};
  const mode = p.get('mockAi');
  if (mode === 'setup' || mode === 'off' || mode === 'ready') o.mode = mode;
  const step = p.get('mockAiStepMs');
  if (step !== null && /^\d{1,4}$/.test(step)) o.stepMs = Number(step);
  const fail = (p.get('mockAiFail') ?? '')
    .split(',')
    .filter((f): f is AiFailable => (AI_FAILABLE as readonly string[]).includes(f));
  if (fail.length) o.fail = fail;
  const runtime = p.get('mockAiRuntime');
  if (runtime === 'none' || runtime === 'broken' || runtime === 'installed') o.runtime = runtime;
  const net = p.get('mockAiNet');
  if ((MOCK_NET_FAILURES as readonly (string | null)[]).includes(net)) o.net = net as MockNetFailure;
  const updates = p.get('mockAiUpdates');
  if (updates === 'available' || updates === 'automatic' || updates === 'manual' || updates === 'none') o.updates = updates;
  return o;
}

interface MockChat {
  record: ChatRecord;
  events: ChatEnvelope[];
  nextSeq: number;
  running: boolean;
  cancel: boolean;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** The AI folder of the demo machine (Windows, like the mock hardware). */
const MOCK_AI_DIR = 'C:\\Users\\demo\\AppData\\Roaming\\IMT\\ai';

export class MockAiBackend extends AiBackend {
  private readonly mode: 'ready' | 'setup' | 'off';
  private readonly stepMs: number;
  private readonly fails: Set<AiFailable>;
  private readonly net: MockNetFailure | null;
  private installed: InstalledModel[];
  private activeFile: string | null;
  private backend: string | null;
  private readonly rt: MockRuntimes;
  private readonly updates: MockAiUpdates;
  private setupCancelled = false;
  private setupRunning = false;
  private readonly chats = new Map<string, MockChat>();
  private counter = 0;
  private readonly setupHandlers = new Set<(e: SetupEvent) => void>();
  private readonly statusHandlers = new Set<(s: AiStatus) => void>();
  private readonly chatHandlers = new Set<(e: ChatEventEnvelope) => void>();
  private readonly continueHook = (mockWeb().continueChat = (sessionId: string, title: string) => this.chatSend(sessionId, `I opened "${title}" for you: read it and go on with my question.`, {} as ChatContext));
  /** Every chat send, for tests. */
  readonly sent: { sessionId: string; text: string; context: ChatContext }[] = [];

  constructor(options: MockAiOptions = {}) {
    super();
    this.mode = options.mode ?? 'ready';
    this.stepMs = options.stepMs ?? 25;
    this.fails = new Set(options.fail ?? []);
    this.net = options.net ?? null;
    const ready = this.mode === 'ready';
    this.installed = ready ? [installedFrom(catalogModel(1), true)] : [];
    this.activeFile = ready ? catalogModel(1).file : null;
    this.rt = new MockRuntimes(
      options.runtime ?? 'installed',
      this.stepMs,
      this.net
        ? mockNetError(this.net, 'github.com')
        : this.fails.has('runtime')
          ? new AiError('network', 'network error: the connection to github.com was reset')
          : null,
    );
    this.backend = ready ? (this.rt.backendName() ?? 'Vulkan') : null;
    this.updates = new MockAiUpdates(this.mode === 'off' ? 'none' : (options.updates ?? 'none'), this.stepMs, this.net);
  }

  private check(op?: AiFailable): void {
    if (this.mode === 'off')
      throw new AiError('unavailable', 'The AI engine is not installed in this build of IMT.');
    if (op && this.fails.has(op))
      throw new AiError('engine', `The model engine failed: simulated ${op} failure.`);
  }

  private needModel(op: AiFailable): void {
    this.check(op);
    if (!this.activeFile)
      throw new AiError(
        'noModel',
        'No AI model is set up yet: choose "Set up AI assistant" first.',
      );
  }

  /** A use case as an AI job (one at a time, visible in the Jobs pane), like the engine; no model fails at once. */
  private aiJob<T>(op: AiFailable, title: string, link: JobLink | null, f: () => T, result: (v: T) => JobResult | null, doneTitle?: string): Promise<T> {
    if (this.mode !== 'ready' || !this.activeFile)
      return this.later(() => {
        this.needModel(op);
        return f();
      }, 0);
    return mockJobs()
      .run({ kind: 'ai', title, link, doneTitle }, async (ctx) => {
        await mockSteps(ctx, this.stepMs * 4, 4);
        this.needModel(op);
        const value = f();
        return { value, result: result(value) };
      })
      .catch((e: unknown) => {
        throw e instanceof MockJobCancelled ? new AiError('cancelled', 'cancelled') : e;
      });
  }

  private async later<T>(f: () => T, ms = this.stepMs * 4): Promise<T> {
    await sleep(ms);
    return f();
  }

  private currentStatus(): AiStatus {
    if (this.mode === 'off') {
      return {
        available: false,
        state: 'unavailable',
        reason: 'The AI engine is not installed in this build of IMT.',
        model: null,
        backend: null,
        rssBytes: 0,
        notes: [],
        safeMode: false,
        setupRunning: false,
      };
    }
    return {
      available: true,
      state: this.activeFile ? 'ready' : 'noModel',
      reason: null,
      model: this.activeFile,
      backend: this.backend,
      rssBytes: this.activeFile ? 3_400_000_000 : 0,
      notes: [],
      safeMode: false,
      setupRunning: this.setupRunning,
    };
  }

  private emitStatus(): void {
    const s = this.currentStatus();
    for (const h of this.statusHandlers) h(s);
  }

  private emitSetup(e: SetupEvent): void {
    for (const h of this.setupHandlers) h(e);
  }

  // ---- status and set-up ----

  status() {
    return this.later(() => this.currentStatus(), 0);
  }
  hardware() {
    return this.later(() => {
      this.check();
      return structuredClone(MOCK_HARDWARE);
    });
  }
  recommend() {
    return this.later(() => {
      this.check();
      return mockRecommendations(this.installed.map((m) => m.fileName));
    });
  }
  catalog() {
    return this.later(() => {
      this.check();
      return mockRecommendations(this.installed.map((m) => m.fileName)).ranked;
    });
  }
  setupPlan(modelId?: string | null) {
    return this.later(() => {
      this.check();
      const recs = mockRecommendations(this.installed.map((m) => m.fileName));
      const chosen =
        (modelId ? recs.ranked.find((r) => r.model.id === modelId) : recs.best) ?? null;
      if (modelId && !chosen)
        throw new AiError('notFound', `'${modelId}' is not in the model list`);
      return {
        hardware: structuredClone(MOCK_HARDWARE),
        recommendations: recs,
        chosen,
        license: chosen?.model.license ?? null,
        gated: false,
        downloadBytes: chosen && !chosen.installed ? chosen.model.sizeBytes : 0,
        alreadyInstalled: !!chosen?.installed,
        note: null,
      };
    });
  }

  async setupRun(modelId: string): Promise<void> {
    this.check();
    const m = MOCK_CATALOG.find((c) => c.id === modelId);
    if (!m) throw new AiError('notFound', `'${modelId}' is not in the model list`);
    if (this.setupRunning)
      throw new AiError('busy', 'A model is already being set up; wait for it or cancel it first.');
    this.setupRunning = true;
    this.setupCancelled = false;
    void this.runSetup(m.file, !this.installed.some((i) => i.fileName === m.file), m.sizeBytes);
  }

  private async runSetup(file: string, download: boolean, size: number): Promise<void> {
    const end = (e: SetupEvent) => {
      this.setupRunning = false;
      this.emitSetup(e);
      this.emitStatus();
    };
    const step = async (e: SetupEvent) => {
      if (this.setupCancelled) throw new AiError('cancelled', 'cancelled');
      this.emitSetup(e);
      await sleep(this.stepMs * 2);
    };
    try {
      await step({ stage: 'probing' });
      await step({ stage: 'recommending' });
      if (download) {
        const steps = 8;
        for (let i = 0; i <= steps; i++) {
          const bytes = Math.round((size * i) / steps);
          await step({
            stage: 'downloading',
            bytes,
            total: size,
            bytesPerSec: 48_000_000,
            etaSecs: Math.ceil((size - bytes) / 48_000_000),
          });
        }
        if (this.net) throw mockNetError(this.net, 'huggingface.co');
        if (this.fails.has('setup'))
          throw new AiError('network', 'network error: the connection to the model host was reset');
        await step({ stage: 'verifying', bytes: size, total: size });
        this.installed = [
          ...this.installed,
          installedFrom(
            MOCK_CATALOG.find((c) => c.file === file)!,
            false,
          ),
        ];
      }
      await step({ stage: 'validating' });
      await step({ stage: 'loading', gpu: true });
      if (!this.rt.hasRuntime())
        throw new AiError(
          'noRuntime',
          'no inference runtime is installed: download one in the AI set-up',
        );
      await step({ stage: 'warmingUp' });
      const backend = this.rt.backendName() ?? 'CPU';
      this.activeFile = file;
      this.backend = backend;
      this.installed = this.installed.map((i) => ({ ...i, active: i.fileName === file }));
      end({ stage: 'ready', model: file, backend });
    } catch (e) {
      const err = e instanceof AiError ? e : new AiError('engine', String(e));
      end(
        err.code === 'cancelled'
          ? { stage: 'cancelled' }
          : { stage: 'failed', code: err.code, message: err.message },
      );
    }
  }

  setupCancel() {
    return this.later(() => {
      if (!this.setupRunning) return false;
      this.setupCancelled = true;
      return true;
    }, 0);
  }

  models() {
    return this.later(() => {
      this.check('models');
      return structuredClone(this.installed);
    });
  }

  async activateModel(fileName: string): Promise<void> {
    this.check();
    if (!this.installed.some((m) => m.fileName === fileName))
      throw new AiError('notFound', `${fileName} is not installed`);
    if (this.setupRunning)
      throw new AiError('busy', 'A model is already being set up; wait for it or cancel it first.');
    this.setupRunning = true;
    this.setupCancelled = false;
    void this.runSetup(fileName, false, 0);
  }

  deleteModel(fileName: string) {
    return this.later(() => {
      this.check();
      if (!this.installed.some((m) => m.fileName === fileName))
        throw new AiError('notFound', `not found: ${fileName}`);
      this.installed = this.installed.filter((m) => m.fileName !== fileName);
      if (this.activeFile === fileName) {
        this.activeFile = null;
        this.backend = null;
      }
      this.emitStatus();
    });
  }

  addLocalModel(path: string) {
    return this.later(() => {
      this.check();
      if (!/\.gguf$/i.test(path)) throw new AiError('invalid', 'model files end in .gguf');
      const name = (path.split(/[\\/]/).pop() ?? 'model.gguf').replace(/[^A-Za-z0-9.+_-]/g, '_');
      // A catalog model downloaded by hand is "verified" and filed under its catalog name, like the engine.
      const known = MOCK_CATALOG.find((c) => c.file.toLowerCase() === name.toLowerCase());
      const m: InstalledModel = known
        ? installedFrom(known, false)
        : {
            ...installedFrom(catalogModel(2), false),
            fileName: name,
            name: name.replace(/\.gguf$/i, ''),
            catalogId: null,
            license: null,
          };
      this.installed = [...this.installed.filter((i) => i.fileName !== name), m];
      return structuredClone(m);
    });
  }

  reset() {
    return this.later(() => this.currentStatus(), 0);
  }

  // ---- inference runtimes ----

  runtimes() {
    return this.later(() => {
      this.check();
      return this.rt.overview();
    });
  }
  async runtimeInstall(id: string): Promise<void> {
    this.check();
    this.rt.install(id);
  }
  async runtimeAddArchives(id: string, paths: string[]): Promise<void> {
    this.check();
    this.rt.installFrom(id, paths);
  }
  runtimeCancel() {
    return this.later(() => this.rt.cancel(), 0);
  }

  // ---- update checks ----

  updatesStatus() {
    return this.later(() => {
      this.check();
      return this.updates.status();
    }, 0);
  }
  async updatesCheck(kind?: AiUpdateKind | null): Promise<AiUpdatesStatus> {
    this.check();
    return this.updates.check(kind);
  }
  async updatesApply(key: string): Promise<AiUpdatesStatus> {
    this.check();
    return this.updates.install(key);
  }
  updatesSkip(key: string) {
    return this.later(() => this.updates.skip(key), 0);
  }
  updatesLater(key: string) {
    return this.later(() => this.updates.later(key), 0);
  }

  // ---- manual downloads ----

  manualDownload(kind: ManualKind, id: string) {
    return this.later((): ManualDownload => {
      this.check();
      if (kind === 'runtime') {
        const b = this.rt.overview().builds.find((x) => x.id === id);
        return {
          kind,
          id,
          name: b?.label ?? id,
          files: this.rt.manualFiles(id),
          folder: `${MOCK_AI_DIR}\\runtimes\\b11669\\${id}`,
        };
      }
      const m = MOCK_CATALOG.find((c) => c.id === id);
      if (!m) throw new AiError('notFound', `'${id}' is not in the model list`);
      return {
        kind,
        id,
        name: m.name,
        files: [
          {
            name: m.file,
            url: `https://huggingface.co/${m.hfRepo}/resolve/main/${m.file}?download=true`,
            sizeBytes: m.sizeBytes,
            sha256: m.sha256,
          },
        ],
        folder: `${MOCK_AI_DIR}\\models`,
      };
    }, 0);
  }
  openManualDownload(kind: ManualKind, id: string, index: number) {
    return this.manualDownload(kind, id).then((d) => {
      const f = d.files[index];
      if (!f) throw new AiError('invalid', 'that file is not part of this download');
      this.opened.push(f.url);
    });
  }
  runtimeDelete(tag: string, id: string) {
    return this.later(() => {
      this.check();
      const wasActive = this.rt.active() === id;
      this.rt.delete(tag, id);
      if (wasActive) this.backend = this.rt.backendName();
      this.emitStatus();
    });
  }
  runtimeChoose(id: string) {
    return this.later(() => {
      this.check();
      this.rt.choose(id);
      if (this.activeFile) this.backend = this.rt.backendName();
      this.emitStatus();
    });
  }

  // ---- use cases ----

  proposeRegex(lines: string[]) {
    if (!lines.length) return Promise.reject(new AiError('invalid', 'give 1 to 50 example lines'));
    return this.aiJob('propose', 'Suggest a regex', null, () => mockRegexProposal(lines), proposal);
  }
  proposeProfile(source: SampleSource) {
    return this.aiJob('propose', 'Propose a parsing profile', eventLink(source), () => mockProfileProposal(), proposal);
  }
  proposePipeline(goal: string, source?: SampleSource) {
    return this.aiJob('propose', `Propose pipeline steps: ${goal}`, eventLink(source), () => mockPipelineProposal(goal), proposal);
  }
  proposeMarkers(goal: string, source?: SampleSource) {
    return this.aiJob('propose', `Propose markers: ${goal}`, eventLink(source), () => mockMarkersProposal(goal), proposal);
  }
  proposeExtractors(goal: string, source?: SampleSource) {
    return this.aiJob('propose', `Propose extractors: ${goal}`, eventLink(source), () => mockExtractorsProposal(goal), proposal);
  }
  nlToQuery(question: string) {
    return this.aiJob('nlToQuery', `Write a query: ${question.trim().slice(0, 60)}`, null, (): QueryProposal => {
      const q = question.toLowerCase();
      const ql = q.includes('warn')
        ? 'level = warning'
        : q.includes('web02')
          ? 'MachineName = WEB02 and level >= error'
          : 'level >= error';
      return {
        kind: 'query',
        ql,
        explanation: `Events matching "${question.trim()}": ${ql}.`,
        estimatedMatches: 37,
        repaired: false,
      };
    }, proposal);
  }
  explainEvent(recordId: number) {
    return this.aiJob('explain', `Explain event ${recordId}`, { type: 'event', recordId }, (): Narrative => {
      return {
        text: `Event ${recordId} is a failed order submission: the payment gateway timed out after 30 s.\n\n- **Likely cause:** the gateway pool was exhausted\n- **Next step:** check \`level >= error\` around it`,
        citations: [
          { kind: 'record', id: recordId, verified: true },
          { kind: 'query', ql: 'level >= error' },
        ],
        evidence: { recordIds: [recordId], queries: ['level >= error'] },
        usage: { inputTokens: 900, outputTokens: 80 },
      };
    }, narrative, 'Explanation ready');
  }
  summarize(from: number, to: number) {
    return this.aiJob('summarize', 'Summarize a time range', { type: 'range', from, to }, (): Narrative => {
      const { fatal } = demoRecordIds();
      return {
        text: `Between ${new Date(from).toISOString().slice(11, 16)} and ${new Date(to).toISOString().slice(11, 16)} UTC, WEB02 ran out of memory and crashed (event ${fatal}); it restarted a minute later.`,
        citations: [{ kind: 'record', id: fatal, verified: true }],
        evidence: { recordIds: [fatal], queries: [] },
        usage: { inputTokens: 1200, outputTokens: 60 },
      };
    }, narrative, 'Summary ready');
  }
  rootCauses() {
    return this.aiJob('rootCauses', 'Find likely root causes', null, (): Narrative => {
      const { fatal, error } = demoRecordIds();
      return {
        text: `1. **Memory leak on WEB02** led to an OutOfMemoryException (event ${fatal}).\n2. **Payment gateway timeouts** caused failed orders (event ${error}); see \`level >= error\`.`,
        citations: [
          { kind: 'record', id: fatal, verified: true },
          { kind: 'record', id: error, verified: true },
          { kind: 'query', ql: 'level >= error' },
        ],
        evidence: { recordIds: [fatal, error], queries: ['level >= error'] },
        usage: { inputTokens: 2400, outputTokens: 120 },
      };
    }, narrative, 'Root causes ready');
  }
  /** "Research this" / "Research…": a research job whose answer becomes a chat turn, or a search text to confirm. */
  async research(args: ResearchArgs): Promise<ResearchStarted> {
    this.needModel('chat');
    const question = (args.question ?? '').trim() || (args.recordId !== null && args.recordId !== undefined ? 'What does this error mean and how can it be fixed: System.UnauthorizedAccessException 0x80070005' : '');
    if (!question) throw new AiError('invalid', 'give the question to research');
    if (args.sessionId && !this.chats.has(args.sessionId)) throw new AiError('notFound', 'this chat no longer exists');
    const query = (args.query ?? '').trim() || question;
    if (mockLeaks(query) && !args.confirmed) {
      const segments = query.split(/(\bWEB0\d\b)/i).filter((t) => t).map((t) => ({ text: t, leak: mockLeaks(t) ? ('logValue' as const) : null }));
      return { status: 'confirm', question, checked: { query, needsConfirm: true, segments } };
    }
    const max = Math.max(1, Math.min(10, args.maxResults ?? 5));
    const jobId = startMockResearch(question, max, this.stepMs, args.sessionId ?? null, (r) => this.recordAnswer(args.sessionId ?? null, `Research: ${question}`, r.answer, webCitations(r)));
    return { status: 'started', jobId };
  }

  /** Adds an answer as a turn of the chat (a new chat when it is missing or busy); returns the chat. */
  private recordAnswer(sessionId: string | null, user: string, text: string, citations: Citation[]): string {
    let c = sessionId ? this.chats.get(sessionId) : undefined;
    if (!c || c.running) {
      const id = `chat-${++this.counter}`;
      c = { record: { id, title: '', turns: [] }, events: [], nextSeq: 1, running: false, cancel: false };
      this.chats.set(id, c);
    }
    if (!c.record.title) c.record.title = user.slice(0, 60);
    this.emitChat(c, { type: 'done', text, citations, usage: { inputTokens: 0, outputTokens: 0 }, notes: [] });
    c.record.turns.push({ user, assistant: text, citations, proposals: [], web: citations.flatMap((x) => (x.kind === 'web' ? [{ title: x.title, url: x.url }] : [])), cancelled: false, usage: { inputTokens: 0, outputTokens: 0 } } as TurnRecord);
    return c.record.id;
  }

  /** Every use case as a background AI job: resolves with the job's id at once, or rejects when it cannot start. */
  async start(request: AiStartRequest): Promise<number> {
    const before = mockJobs().nextId();
    const answer: Promise<unknown> = this.useCase(request);
    if (mockJobs().nextId() === before) {
      // No job: it did not start (no model, invalid arguments); say why.
      await answer;
      throw new AiError('engine', 'the request did not start');
    }
    answer.catch(() => undefined);
    return before;
  }

  private useCase(r: AiStartRequest): Promise<unknown> {
    switch (r.op) {
      case 'explain':
        return this.explainEvent(r.recordId);
      case 'summarize':
        return this.summarize(r.from, r.to);
      case 'rootCauses':
        return this.rootCauses();
      case 'regex':
        return this.proposeRegex(r.lines);
      case 'profile':
        return this.proposeProfile(r);
      case 'pipeline':
        return this.proposePipeline(r.goal, r);
      case 'markers':
        return this.proposeMarkers(r.goal, r);
      case 'extractors':
        return this.proposeExtractors(r.goal, r);
      case 'query':
        return this.nlToQuery(r.question);
      case 'folderSetup':
        return Promise.reject(new AiError('invalid', 'the folder set-up starts its own AI job'));
    }
  }

  /** Stops the window's use cases (running and waiting), not its chat turns. */
  cancel() {
    return this.later(() => mockJobs().cancelWhere((j) => j.kind === 'ai' && j.source.link?.type !== 'chat'), 0);
  }

  // ---- chat ----

  private info(c: MockChat): SessionInfo {
    return {
      id: c.record.id,
      title: c.record.title,
      turns: c.record.turns.length,
      running: c.running,
    };
  }

  private chat(id: string): MockChat {
    const c = this.chats.get(id);
    if (!c) throw new AiError('notFound', 'this chat no longer exists');
    return c;
  }

  chatSessions() {
    return this.later(() => {
      this.check();
      return [...this.chats.values()].map((c) => this.info(c));
    }, 0);
  }
  chatNew() {
    return this.later(() => {
      this.check();
      const id = `chat-${++this.counter}`;
      const c: MockChat = {
        record: { id, title: '', turns: [] },
        events: [],
        nextSeq: 1,
        running: false,
        cancel: false,
      };
      this.chats.set(id, c);
      return this.info(c);
    }, 0);
  }
  chatGet(sessionId: string) {
    return this.later(() => structuredClone(this.chat(sessionId).record), 0);
  }
  chatDelete(sessionId: string) {
    return this.later(() => {
      const c = this.chats.get(sessionId);
      if (c) c.cancel = true;
      mockWeb().forget(sessionId);
      return this.chats.delete(sessionId);
    }, 0);
  }

  /** Starts the turn as an AI job (it may wait behind other AI requests); resolves with the job's id. */
  async chatSend(sessionId: string, text: string, context: ChatContext): Promise<number> {
    this.needModel('chat');
    const c = this.chat(sessionId);
    if (c.running)
      throw new AiError('busy', 'This chat is still answering; wait for it or stop it first.');
    if (!text.trim()) throw new AiError('invalid', 'the question is empty');
    this.sent.push({ sessionId, text, context });
    c.running = true;
    c.cancel = false;
    const question = text.trim();
    const job = mockJobs().start({ kind: 'ai', title: `Chat: ${question.slice(0, 60)}`, link: { type: 'chat', sessionId } }, async (ctx) => {
      const stop = setInterval(() => {
        if (ctx.cancelled()) c.cancel = true;
      }, 20);
      try {
        await this.runTurn(c, question);
      } finally {
        clearInterval(stop);
      }
      return { value: undefined, result: { type: 'chat', sessionId } };
    });
    job.done.catch(() => {
      // Cancelled while waiting: the chat is free again and told.
      if (c.running) {
        c.running = false;
        this.emitChat(c, { type: 'error', message: 'Stopped before it started.', cancelled: true });
      }
    });
    return job.id;
  }

  private emitChat(c: MockChat, event: ChatEvent): void {
    const env = { ...event, seq: c.nextSeq++ } as ChatEnvelope;
    c.events.push(env);
    const out = { ...env, sessionId: c.record.id } as ChatEventEnvelope;
    for (const h of this.chatHandlers) h(out);
  }

  private async stream(c: MockChat, text: string): Promise<boolean> {
    for (const piece of text.match(/.{1,12}/gs) ?? []) {
      if (c.cancel) return false;
      this.emitChat(c, { type: 'delta', text: piece });
      await sleep(this.stepMs);
    }
    return !c.cancel;
  }

  private async runTurn(c: MockChat, text: string): Promise<void> {
    const q = text.toLowerCase();
    const ids = demoRecordIds();
    let answer = '';
    const citations: Citation[] = [];
    const proposals: Proposal[] = [];
    const web: { title: string; url: string }[] = [];
    const tool = async (
      name: string,
      args: string,
      summary: string,
      recordIds: number[] = [],
      queries: string[] = [],
      error = false,
    ) => {
      this.emitChat(c, { type: 'toolStarted', name, args });
      await sleep(this.stepMs * 2);
      this.emitChat(c, {
        type: 'toolFinished',
        name,
        summary,
        error,
        evidence: { recordIds, queries },
      });
    };
    const finish = (ok: boolean, partial: string) => {
      c.running = false;
      if (!c.record.title) c.record.title = text.slice(0, 60);
      if (ok) {
        this.emitChat(c, {
          type: 'done',
          text: answer,
          citations,
          usage: { inputTokens: 1500, outputTokens: Math.ceil(answer.length / 4) },
          notes: [],
        });
        c.record.turns.push({
          user: text,
          assistant: answer,
          citations,
          proposals,
          web,
          cancelled: false,
          usage: { inputTokens: 1500, outputTokens: Math.ceil(answer.length / 4) },
        });
      } else {
        this.emitChat(c, { type: 'error', message: 'cancelled', cancelled: true });
        if (partial.trim())
          c.record.turns.push({
            user: text,
            assistant: partial,
            citations: [],
            proposals: [],
            web: [],
            cancelled: true,
            usage: { inputTokens: 0, outputTokens: 0 },
          } as TurnRecord);
      }
    };
    try {
      await sleep(this.stepMs);
      const browsed = await mockBrowserTurn({
        sessionId: c.record.id,
        question: text,
        web: mockWeb(),
        emit: (e) => this.emitChat(c, e),
        tool: (name, args, summary, error = false) => tool(name, args, summary, [], [], error),
        cancelled: () => c.cancel,
        citations,
        sources: web,
      });
      if (browsed !== null) {
        answer = browsed;
      } else if (wantsResearch(text) && mockWeb().researchAuto) {
        // Research mode: the model calls web_research; the research runs as a job (Jobs pane) and the turn waits.
        this.emitChat(c, { type: 'toolStarted', name: 'web_research', args: JSON.stringify({ question: text }) });
        const report = await mockJobs().run({ kind: 'research', title: `Research: ${text.slice(0, 60)}`, link: { type: 'chat', sessionId: c.record.id } }, async (ctx) => ({
          value: await runMockResearch(ctx, text, 5, this.stepMs),
          result: { type: 'chat', sessionId: c.record.id } as JobResult,
        }));
        this.emitChat(c, { type: 'toolFinished', name: 'web_research', summary: `read ${report.sources.length} of 5 results`, error: false, evidence: { recordIds: [], queries: [] } });
        answer = report.answer;
        for (const s of report.sources) {
          citations.push({ kind: 'web', title: s.title, url: s.url });
          web.push(s);
        }
      } else if (wantsResearch(text)) {
        // Automatic research is off: the model offers the buttons instead of guessing.
        this.emitChat(c, { type: 'researchOffer', question: text, query: text, segments: [], needsConfirm: false });
        answer = 'This needs information from the web. Use **Research this** below, or let the AI drive the browser for this chat.';
      } else if (q.includes('regex')) {
        await tool(
          'propose_regex',
          '{"example_lines":["…"]}',
          'Regular expression ready to review',
        );
        const p = mockRegexProposal([
          '2026-10-14 09:00:02 [INF] Worker started',
          '2026-10-14 09:00:03 [WRN] Queue depth 812',
        ]);
        proposals.push(p);
        this.emitChat(c, { type: 'proposal', proposal: p });
        answer =
          'I prepared a regular expression for these lines; review it and apply it as a highlight rule.';
      } else if (q.includes('pipeline') || q.includes('drop')) {
        await tool(
          'propose_pipeline',
          '{"goal":"drop health checks"}',
          'Pipeline steps ready to review',
        );
        const p = mockPipelineProposal('drop health checks');
        proposals.push(p);
        this.emitChat(c, { type: 'proposal', proposal: p });
        answer =
          'I prepared a pipeline step that drops health checks. Nothing changes until you apply it.';
      } else if (q.includes('marker')) {
        await tool('propose_markers', '{"goal":"timeouts"}', 'Markers ready to review');
        const p = mockMarkersProposal('timeouts');
        proposals.push(p);
        this.emitChat(c, { type: 'proposal', proposal: p });
        answer = 'I prepared a marker for timeouts.';
      } else if (q.includes('extract')) {
        await tool('propose_extractors', '{"goal":"durations"}', 'Extractors ready to review');
        const p = mockExtractorsProposal('durations');
        proposals.push(p);
        this.emitChat(c, { type: 'proposal', proposal: p });
        answer = 'I prepared an extractor for request durations.';
      } else if (q.includes('query')) {
        await tool('propose_query', '{"question":"errors on WEB02"}', 'Query ready to review');
        const p: QueryProposal = {
          kind: 'query',
          ql: 'MachineName = WEB02 and level >= error',
          explanation: 'Errors and worse on WEB02.',
          estimatedMatches: 14,
          repaired: false,
        };
        proposals.push(p);
        this.emitChat(c, { type: 'proposal', proposal: p });
        answer = 'Here is a query for errors on WEB02: `MachineName = WEB02 and level >= error`.';
        citations.push({ kind: 'query', ql: p.ql });
      } else if (q.includes('unknown') || q.includes('kubernetes')) {
        await tool(
          'run_query',
          '{"ql":"message contains \\"kubernetes\\""}',
          '0 events',
          [],
          ['message contains "kubernetes"'],
        );
        answer = 'Not found in the logs: no event mentions that.';
      } else {
        await tool(
          'run_query',
          '{"ql":"level = fatal"}',
          '1 events',
          [ids.fatal],
          ['level = fatal'],
        );
        answer = `WEB02 crashed with an OutOfMemoryException (event ${ids.fatal}) after its working set grew for an hour.\n\n- Errors before the crash: \`level >= error\`\n- It restarted about a minute later\n\n\`\`\`\nSystem.OutOfMemoryException: Insufficient memory\n\`\`\``;
        citations.push(
          { kind: 'record', id: ids.fatal, verified: true },
          { kind: 'query', ql: 'level >= error' },
        );
      }
      if (c.cancel) return finish(false, '');
      const ok = await this.stream(c, answer);
      finish(ok, ok ? answer : answer.slice(0, 20));
    } catch {
      c.running = false;
      this.emitChat(c, {
        type: 'error',
        message: 'The assistant stopped unexpectedly.',
        cancelled: false,
      });
    }
  }

  chatCancel(sessionId: string) {
    return this.later(() => {
      const c = this.chats.get(sessionId);
      if (!c?.running) return false;
      c.cancel = true;
      mockJobs().cancelWhere((j) => j.state === 'queued' && j.source.link?.type === 'chat' && j.source.link.sessionId === sessionId);
      mockWeb().denyAll(sessionId);
      return true;
    }, 0);
  }
  chatEventsSince(sessionId: string, seq: number) {
    return this.later(() => {
      const c = this.chats.get(sessionId);
      return c ? structuredClone(c.events.filter((e) => e.seq > seq)) : null;
    }, 0);
  }
  chatHistory() {
    return this.later(
      (): ChatHistoryRecord => ({
        version: 1,
        sessions: [...this.chats.values()].map((c) => structuredClone(c.record)),
      }),
      0,
    );
  }
  chatRestore(record: unknown) {
    return this.later((): RestoreResult => {
      this.chats.clear();
      const r = record as ChatHistoryRecord | null | undefined;
      if (r == null) return { sessions: [], warning: null };
      if (typeof r !== 'object' || typeof r.version !== 'number' || !Array.isArray(r.sessions))
        return { sessions: [], warning: 'The saved chats are damaged and were not loaded.' };
      if (r.version > 1)
        return {
          sessions: [],
          warning: 'The saved chats were written by a newer IMT and were not loaded.',
        };
      for (const s of r.sessions) {
        this.chats.set(s.id, {
          record: structuredClone(s),
          events: [],
          nextSeq: 1,
          running: false,
          cancel: false,
        });
        const n = Number(s.id.replace('chat-', ''));
        if (Number.isFinite(n)) this.counter = Math.max(this.counter, n);
      }
      return { sessions: [...this.chats.values()].map((c) => this.info(c)), warning: null };
    }, 0);
  }

  private readonly written = new Set<string>();
  chatExportMarkdown(sessionId: string, path: string, overwrite = false) {
    return this.later(() => {
      this.chat(sessionId);
      if (this.written.has(path) && !overwrite)
        throw new AiError('fileExists', `${path}${FILE_EXISTS_SUFFIX}`);
      this.written.add(path);
      return path;
    }, 0);
  }
  suggestedPrompts() {
    return this.later(() => {
      this.check();
      return [
        'What happened around 10:42 (OrderService crashed)?',
        'Why is "Payment gateway timeout" happening and what should I do?',
        'Which instance has the most errors?',
        'What should I look at first?',
      ];
    }, 0);
  }
  /** Links "opened" in the system browser (the browser build never navigates away). */
  readonly opened: string[] = [];
  openLink(url: string) {
    return this.later(() => {
      if (!/^https:\/\//.test(url))
        throw new AiError('refused', 'refused: only https links to public sites are opened');
      this.opened.push(url);
    }, 0);
  }
  private subscribe<T>(set: Set<(e: T) => void>, h: (e: T) => void): Promise<Unlisten> {
    set.add(h);
    return Promise.resolve(() => set.delete(h));
  }
  onSetup(handler: (e: SetupEvent) => void) {
    return this.subscribe(this.setupHandlers, handler);
  }
  onRuntime(handler: (e: AiRuntimeEvent) => void) {
    return Promise.resolve(this.rt.subscribe(handler));
  }
  onStatus(handler: (s: AiStatus) => void) {
    return this.subscribe(this.statusHandlers, handler);
  }
  onChat(handler: (e: ChatEventEnvelope) => void) {
    return this.subscribe(this.chatHandlers, handler);
  }
  onUpdates(handler: (s: AiUpdatesStatus) => void) {
    return Promise.resolve(this.updates.subscribe(handler));
  }

  /** Test hook: the code an operation rejects with. */
  static codeOf(e: unknown): AiErrorCode | null {
    return e instanceof AiError ? e.code : null;
  }
}

/** View of a proposal job opens the proposal to review. */
function proposal(p: unknown): JobResult {
  return { type: 'proposal', proposal: p };
}

/** View of an answer job shows the answer with its citations. */
function narrative(n: unknown): JobResult {
  return { type: 'narrative', narrative: n };
}

function eventLink(source?: SampleSource): JobLink | null {
  return source && 'recordId' in source && typeof source.recordId === 'number' ? { type: 'event', recordId: source.recordId } : null;
}
