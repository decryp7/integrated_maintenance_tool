/**
 * Wire contract of the local AI assistant (`app/src-tauri/src/ai/**`, `crates/imt-ai`), camelCase JSON.
 * Commands: `ai_*` (see `ai-backend.ts`). Events: `imt://ai-setup` (`SetupEvent`, all windows),
 * `imt://ai-status` (`AiStatus`, all windows), `imt://ai-chat` (`ChatEventEnvelope`, the chat's window),
 * `imt://ai-runtime` (`AiRuntimeEvent`, all windows), `imt://ai-updates` (`AiUpdatesStatus`, all windows).
 * Every command rejects with `{ code, message }` (`AiErrorPayload`).
 */
import { AgentApprovalRequest, HandOff, PauseReason, QuerySource, Segment } from '../browser/browser.types';
import { ExplainNode, InferredGroup, Level, ParsedPreview } from '../core/api.types';

export const AI_SETUP_EVENT = 'imt://ai-setup';
export const AI_STATUS_EVENT = 'imt://ai-status';
export const AI_CHAT_EVENT = 'imt://ai-chat';
export const AI_RUNTIME_EVENT = 'imt://ai-runtime';
export const AI_UPDATES_EVENT = 'imt://ai-updates';

export type AiErrorCode =
  | 'unavailable'
  | 'noModel'
  | 'noRuntime'
  /** The active model cannot read images: install its image projector or choose a "Text + images" model. */
  | 'noVision'
  | 'busy'
  | 'cancelled'
  | 'invalid'
  | 'engine'
  /** The research or agent browser failed (not open, a page that did not answer or changed). */
  | 'browser'
  | 'contextOverflow'
  | 'malformed'
  | 'toolLoopLimit'
  | 'modelFile'
  | 'memory'
  | 'network'
  /** The server certificate could not be verified (a TLS-inspecting proxy such as Zscaler). */
  | 'tls'
  /** The proxy asks for a sign-in (HTTP 407). */
  | 'proxyAuth'
  /** The configured download proxy cannot be used. */
  | 'proxy'
  | 'offline'
  | 'timeout'
  /** The server (or a proxy in between) answered with an unexpected HTTP status. */
  | 'httpStatus'
  | 'gated'
  | 'notFound'
  | 'integrity'
  | 'disk'
  | 'refused'
  | 'io'
  | 'fileExists';

export interface AiErrorPayload {
  code: AiErrorCode;
  message: string;
}

// ---- engine status, hardware, catalog ----

export type EngineState =
  'unavailable' | 'disabled' | 'noModel' | 'idle' | 'loading' | 'ready' | 'failed';

export interface AiStatus {
  available: boolean;
  state: EngineState;
  reason: string | null;
  model: string | null;
  backend: string | null;
  rssBytes: number;
  notes: string[];
  safeMode: boolean;
  setupRunning: boolean;
  /** The active model reads images ("Text + images"): its projector is installed (ADR 0025). */
  vision: boolean;
}

export interface Gpu {
  name: string;
  vramGb: number | null;
  unified: boolean;
}

export interface Hardware {
  os: string;
  arch: string;
  cpuCores: number;
  totalRamGb: number;
  availableRamGb: number;
  gpus: Gpu[];
}

export type Fit = 'comfortable' | 'tight' | 'tooLarge';
export type Speed = 'fast' | 'moderate' | 'slow';

export interface Assessment {
  needGb: number;
  fit: Fit;
  speed: Speed;
}

export interface CatalogModel {
  id: string;
  name: string;
  family: string;
  paramsB: number;
  activeParamsB?: number | null;
  quant: string;
  hfRepo: string;
  file: string;
  sha256: string;
  sizeBytes: number;
  license: string;
  toolCalling: boolean;
  contextDefault: number;
  minVramGb: number;
  minRamGb: number;
  moe: boolean;
  recommendedFor: string[];
  notes: string;
  /** Older files of this entry it replaces (an earlier revision or quantisation); absent when none. */
  supersedes?: string[];
  /** The model's image projector (`mmproj`, same repository): present when it reads images (ADR 0025). */
  projector?: CatalogProjector | null;
}

/** A model's multimodal projector as the catalog pins it (`imt_ai::catalog::CatalogProjector`). */
export interface CatalogProjector {
  file: string;
  sha256: string;
  sizeBytes: number;
}

export interface Recommendation {
  model: CatalogModel;
  assessment: Assessment;
  installed: boolean;
  context: number;
}

export interface Recommendations {
  hardware: string;
  budget: { usableGb: number; class: 'discreteGpu' | 'unified' | 'cpuOnly' };
  best: Recommendation | null;
  fastest: Recommendation | null;
  ranked: Recommendation[];
  message: string;
}

export interface SetupPlan {
  hardware: Hardware;
  recommendations: Recommendations;
  chosen: Recommendation | null;
  license: string | null;
  gated: boolean;
  downloadBytes: number;
  alreadyInstalled: boolean;
  note: string | null;
}

/** One stage of a set-up or activation; `failed` and `cancelled` end it like `ready`. */
export type SetupEvent =
  | { stage: 'probing' }
  | { stage: 'recommending' }
  | {
      stage: 'downloading';
      bytes: number;
      total: number;
      bytesPerSec: number;
      etaSecs: number | null;
    }
  | { stage: 'verifying'; bytes: number; total: number }
  | { stage: 'validating' }
  | { stage: 'loading'; gpu: boolean }
  | { stage: 'fellBackToCpu'; reason: string }
  | { stage: 'warmingUp' }
  | { stage: 'ready'; model: string; backend: string }
  | { stage: 'failed'; code: AiErrorCode; message: string }
  | { stage: 'cancelled' };

export interface GgufInfo {
  version: number;
  architecture: string;
  name: string | null;
  quant: string | null;
  contextLength: number | null;
  blockCount: number | null;
  tensorCount: number;
  fileSize: number;
  paramsB: number;
  knownArchitecture: boolean;
}

export interface InstalledModel {
  fileName: string;
  sizeBytes: number;
  info: GgufInfo | null;
  problem: string | null;
  active: boolean;
  assessment: Assessment | null;
  catalogId: string | null;
  name: string;
  license: string | null;
  toolCalling: boolean;
  /** Reads images: its projector is installed and fits it. */
  vision: boolean;
  /** The installed projector (also one that cannot be used, with its problem). */
  projector: InstalledProjector | null;
  /** Bytes of the catalog projector that would add image support, when it is missing. */
  projectorDownloadBytes: number | null;
}

/** What a projector's header says (`imt_ai::gguf::ProjectorInfo`). */
export interface ProjectorInfo {
  name: string | null;
  projectorType: string | null;
  projectionDim: number | null;
  imageSize: number | null;
  patchSize: number | null;
  spatialMergeSize: number | null;
  fileSize: number;
}

/** A model's projector in the models folder (`imt_ai::models::store::InstalledProjector`). */
export interface InstalledProjector {
  fileName: string;
  sizeBytes: number;
  info: ProjectorInfo | null;
  problem: string | null;
}

// ---- inference runtimes (prebuilt llama-server builds, ADR 0009; `imt_ai::runtimes::overview`) ----

/** An accelerator a build reported (`llama-server --list-devices`). */
export interface AiRuntimeDevice {
  /** ggml's device name, e.g. `Vulkan0`, `CUDA0`, `MTL0`. */
  name: string;
  description: string;
  totalMib: number;
  freeMib: number;
}

/** One build the pinned catalog offers for this platform. */
export interface AiRuntimeEntry {
  /** `cpu`, `vulkan`, `cuda-13`, `metal`, ... */
  id: string;
  label: string;
  downloadBytes: number;
  installed: boolean;
  installedBytes: number | null;
  /** `null` when not installed or not checked yet. */
  usable: boolean | null;
  problem: string | null;
  devices: AiRuntimeDevice[];
  /** The build suggested for this machine. */
  suggested: boolean;
  /** The build that runs the model with the current settings. */
  active: boolean;
}

/** A build on disk. */
export interface AiInstalledRuntime {
  id: string;
  tag: string;
  bytes: number;
  /** Belongs to the release IMT pins (older ones can be deleted). */
  current: boolean;
}

export interface AiRuntimeOverview {
  release: string;
  tag: string;
  platform: string;
  /** The `inference_build` setting: `auto` or a build id. */
  choice: string;
  suggested: string | null;
  /** The build in use: the chosen one when it works, else the best working one. */
  active: string | null;
  /** Set when a custom llama-server replaces the downloaded builds. */
  customPath: string | null;
  builds: AiRuntimeEntry[];
  /** Builds of older releases still on disk. */
  outdated: AiInstalledRuntime[];
  /** Nothing is installed (and no custom path is set): offer a download. */
  needsDownload: boolean;
  /** Why the builds could not be checked, if they could not. */
  problem: string | null;
}

// ---- manual downloads (`ai_manual_download`) ----

/** What is downloaded by hand: a catalog model or an inference runtime build. */
export type ManualKind = 'model' | 'runtime';

export interface ManualFile {
  name: string;
  /** The https address the browser downloads it from (computed by the engine from its catalogs). */
  url: string;
  sizeBytes: number;
  /** The expected SHA-256, when known. */
  sha256: string | null;
}

/** The files of a manual download and where IMT keeps them once checked. */
export interface ManualDownload {
  kind: ManualKind;
  id: string;
  /** The model's name or the build's label. */
  name: string;
  files: ManualFile[];
  /** The models folder, or the folder the build is unpacked into. */
  folder: string;
}

/** Progress of a runtime install (`imt://ai-runtime`); `ready`, `failed` and `cancelled` end it. */
export type AiRuntimeEvent =
  | { stage: 'downloading'; id: string; asset: string; index: number; count: number }
  | {
      stage: 'progress';
      id: string;
      bytes: number;
      total: number;
      bytesPerSec: number;
      etaSecs: number | null;
      verifying: boolean;
    }
  | { stage: 'unpacking'; id: string; asset: string }
  | { stage: 'finishing'; id: string }
  | { stage: 'ready'; id: string; runtime: AiInstalledRuntime }
  | { stage: 'failed'; id: string; code: AiErrorCode; message: string }
  | { stage: 'cancelled'; id: string };

// ---- proposals and narratives ----

export interface RegexProposal {
  kind: 'regex';
  pattern: string;
  source: 'model' | 'inferred';
  groups: InferredGroup[];
  matches: boolean[];
  matchCount: number;
  total: number;
  explanation: ExplainNode[];
  rationale: string;
}

export interface ProfileProposal {
  kind: 'profile';
  name: string;
  toml: string;
  importer: string | null;
  parsed: number;
  withTimestamp: number;
  preview: ParsedPreview[];
  rationale: string;
}

export interface PipelineProposal {
  kind: 'pipeline';
  toml: string;
  profileToml: string;
  steps: { index: number; step: string; input: number; output: number }[];
  sampleTotal: number;
  kept: number;
  dropped: number;
  changed: number;
  examples: { before: string; after: string }[];
  rationale: string;
}

export interface MarkersProposal {
  kind: 'markers';
  toml: string;
  profileToml: string;
  markers: { kind: string; label: string; matches: number }[];
  sampled: number;
  rationale: string;
}

export interface ExtractorsProposal {
  kind: 'extractors';
  toml: string;
  profileToml: string;
  extractors: {
    name: string;
    fields: string[];
    matches: number;
    examples: Record<string, unknown>[];
  }[];
  sampled: number;
  rationale: string;
}

export interface QueryProposal {
  kind: 'query';
  ql: string;
  explanation: string;
  estimatedMatches: number | null;
  repaired: boolean;
}

export type Proposal =
  | RegexProposal
  | ProfileProposal
  | PipelineProposal
  | MarkersProposal
  | ExtractorsProposal
  | QueryProposal;
export type ProposalKind = Proposal['kind'];
/** The proposals that change the active profile (applied through the profile's validated save). */
export type ProfileChangeKind = 'pipeline' | 'markers' | 'extractors';

export type Citation =
  | { kind: 'record'; id: number; verified: boolean }
  | { kind: 'query'; ql: string }
  | { kind: 'web'; title: string; url: string };

export interface Evidence {
  recordIds: number[];
  queries: string[];
}

export interface Usage {
  inputTokens: number;
  outputTokens: number;
}

export interface Narrative {
  text: string;
  citations: Citation[];
  evidence: Evidence;
  usage: Usage;
}

// ---- chat ----

/** Where the user is, sent with each turn; the engine validates every value and fetches the details itself. */
export interface ChatContext {
  query?: string | null;
  /** Epoch ms. */
  timeRange?: [number, number] | null;
  selectedIds: number[];
  focusedInstance?: string | null;
  openView?: string | null;
}

export type ChatEvent =
  | { type: 'delta'; text: string }
  | { type: 'toolStarted'; name: string; args: string }
  | { type: 'toolFinished'; name: string; summary: string; error: boolean; evidence: Evidence }
  | { type: 'proposal'; proposal: Proposal }
  /** A search or page the model suggests; nothing loads until the user opens it. */
  | { type: 'linkSuggestion'; kind: 'search' | 'link'; url: string; title: string; query: string | null; segments: Segment[] }
  /** An agent step waits for the user's approval. */
  | { type: 'agentApproval'; request: AgentApprovalRequest }
  /** An agent step starts (one model call each). */
  | { type: 'agentStep'; step: number; limit: number; action: string; site: string | null }
  /** Agent mode stopped and handed the browser back. */
  | { type: 'agentPaused'; reason: PauseReason; steps: number; handOff: HandOff | null }
  /** The model offers web research (or the search text needs confirming): "Research this" and agent mode buttons. */
  | { type: 'researchOffer'; question: string; query: string; segments: Segment[]; needsConfirm: boolean }
  | { type: 'done'; text: string; citations: Citation[]; usage: Usage; notes: string[] }
  | { type: 'error'; message: string; cancelled: boolean };

/** A chat event as replayed (`ai_chat_events_since`): numbered from 1 per chat; 0 = outside the stream. */
export type ChatEnvelope = ChatEvent & { seq: number };
/** A chat event as streamed to the window. */
export type ChatEventEnvelope = ChatEnvelope & { sessionId: string };

export interface SessionInfo {
  id: string;
  title: string;
  turns: number;
  running: boolean;
}

export interface TurnRecord {
  user: string;
  assistant: string;
  citations: Citation[];
  proposals: Proposal[];
  web: { title: string; url: string }[];
  cancelled: boolean;
  usage: Usage;
}

export interface ChatRecord {
  id: string;
  title: string;
  turns: TurnRecord[];
}

/** Saved with the project and the session under the UI state key `chats`. */
export interface ChatHistoryRecord {
  version: number;
  sessions: ChatRecord[];
}

export interface RestoreResult {
  sessions: SessionInfo[];
  warning: string | null;
}

/** Where a proposal's sample comes from: text the user gave, or the lines around an event. */
export interface SampleSource {
  recordId?: number | null;
  sample?: string | null;
  fileName?: string | null;
}

/** Explain, summarise, root causes: an answer that runs in the background ("Explanation ready · View"). */
export type NarrativeRequest = { op: 'explain'; recordId: number } | { op: 'summarize'; from: number; to: number } | { op: 'rootCauses' };

/** `ai_start`: a use case as a background AI job (app/src-tauri/src/ai/requests.rs `AiRequest`, tagged by `op`). */
export type AiStartRequest =
  | NarrativeRequest
  | { op: 'regex'; lines: string[]; intent: string | null }
  | { op: 'profile'; recordId: number | null; sample: string | null; fileName: string | null }
  | { op: 'pipeline' | 'markers' | 'extractors'; goal: string; recordId: number | null; sample: string | null }
  | { op: 'query'; question: string }
  | { op: 'folderSetup'; request: unknown }
  /** "Explain with AI" on an open image; the answer is a narrative. */
  | { op: 'explainImage'; imageId: number; question: string | null }
  /** "Find related logs" for an open image; the answer is a `RelatedOutcome` (job result `relatedLogs`). */
  | { op: 'relatedLogs'; imageId: number };

// ---- images (ADR 0025: app/src-tauri/src/ai/vision.rs, crates/imt-core/src/related, crates/imt-ai/src/vision) ----

export type ImageSourceFormat = 'png' | 'jpeg' | 'bmp' | 'gif' | 'webp';

/** An image opened for the AI (`ai_image_open`, `ai_image_paste`): the clean copy Rust re-encoded, never the file. */
export interface AiImage {
  id: number;
  name: string;
  /** The file it came from; null when pasted or dropped. */
  path: string | null;
  width: number;
  height: number;
  originalWidth: number;
  originalHeight: number;
  sourceFormat: ImageSourceFormat;
  /** `data:image/png;base64,…` (or jpeg) of the clean copy. */
  dataUrl: string;
  /** EXIF capture time as the device wrote it, `YYYY-MM-DD HH:MM:SS`. */
  capturedAt: string | null;
}

export type ScreenKind = 'error_dialog' | 'frozen_window' | 'console' | 'dashboard' | 'log_viewer' | 'browser' | 'other';

/** What the model read off a screenshot (`imt_ai::vision::ScreenFacts`, bounded by the engine). */
export interface ScreenFacts {
  screenKind: ScreenKind;
  windowTitle: string | null;
  application: string | null;
  timestamps: { text: string; hasDate: boolean }[];
  errorTexts: string[];
  errorCodes: string[];
  exceptionNames: string[];
  machineNames: string[];
  userNames: string[];
  serverNames: string[];
  ids: string[];
  versions: string[];
  summary: string;
}

export type ImageTimeSource = 'screen' | 'screenTimeOfDay' | 'exif' | 'fileName' | 'fileModified';

/** When the screenshot was taken, as the engine decided (`imt_core::related::ImageTime`). */
export interface ImageTime {
  atMs: number;
  precisionMs: number;
  source: ImageTimeSource;
  text: string;
}

export interface RelatedEvent {
  id: number;
  ts: number | null;
  level: Level;
  message: string;
  exception: string | null;
  source: number;
  sourceName: string;
  group: string;
  fields: Record<string, unknown>;
}

export type RelatedTitleTarget =
  | { kind: 'source'; name: string }
  | { kind: 'lane'; lane: string }
  | { kind: 'field'; field: string; value: string };

export type RelatedClueKind = 'exceptionName' | 'errorCode' | 'id' | 'errorText' | 'machine' | 'user' | 'server' | 'version';

/** Why an event is linked to the image (`imt_core::related::Reason`). */
export type RelatedReason =
  /** `deltaMs` = event time − image time. */
  | { kind: 'time'; deltaMs: number }
  | { kind: 'title'; text: string; target: RelatedTitleTarget }
  | { kind: 'text'; clue: string; clueKind: RelatedClueKind; partial: boolean };

export interface RelatedLink {
  event: RelatedEvent;
  score: number;
  reasons: RelatedReason[];
}

export type RelatedNote =
  | { kind: 'noTime' }
  | { kind: 'outsideData'; from: number | null; to: number | null }
  /** The clocks likely disagree: offer the clock-offset correction of `sourceId`. */
  | { kind: 'clockDisagreement'; deltaMs: number; sourceId: number; sourceName: string }
  | { kind: 'offsetApplied'; sourceName: string; offsetMs: number }
  | { kind: 'nothingToSearch' };

export interface RelatedResult {
  time: ImageTime | null;
  links: RelatedLink[];
  notes: RelatedNote[];
  examined: number;
}

/** The answer of "find related logs" (job result `relatedLogs`): a typed result notes can link to later. */
export interface RelatedOutcome {
  facts: ScreenFacts;
  result: RelatedResult;
  explanation: Narrative;
}

/** `ai_research` arguments (app/src-tauri/src/browser/research.rs `ResearchArgs`). */
export interface ResearchArgs {
  question?: string | null;
  recordId?: number | null;
  sessionId?: string | null;
  /** 1..10; the setting when absent. */
  maxResults?: number | null;
  /** The search text as the user confirmed or edited it. */
  query?: string | null;
  confirmed?: boolean;
}

/** A search text checked by the leak guard. */
export interface CheckedQuery {
  query: string;
  needsConfirm: boolean;
  segments: Segment[];
}

/** `ai_research` answer: the research job started, or the search text needs the user's confirmation first. */
export type ResearchStarted = { status: 'started'; jobId: number } | { status: 'confirm'; checked: CheckedQuery; question: string };

/** `ai_research_draft` arguments: the question (one about the event when absent) and the event. */
export interface DraftArgs {
  question?: string | null;
  recordId?: number | null;
}

/** `ai_research_draft` answer: the question and the search text the model drafted (or the engine built), checked by the
 * leak guard. The user sees and edits it before `ai_research` runs it. */
export interface ResearchDraft {
  question: string;
  checked: CheckedQuery;
  source: QuerySource;
}

// ---- update checks (app/src-tauri/src/ai/updates.rs, crates/imt-ai/src/updates) ----

/** What IMT does when it finds an update of one kind (Settings → AI). */
export type AiUpdatePolicy = 'automatic' | 'ask' | 'manual';

export type AiUpdateKind = 'runtime' | 'model';

/** One update from the pinned catalogs. */
export interface AiUpdate {
  kind: AiUpdateKind;
  /** Stable per target version: what "Skip this version" remembers. */
  key: string;
  /** The build id or catalog model id (also what "Download manually" takes). */
  id: string;
  name: string;
  /** The installed release tag or model file. */
  from: string;
  /** The release tag or model file it updates to. */
  to: string;
  downloadBytes: number;
  skipped: boolean;
  installing: boolean;
  /** An automatic install waits for the running chat to end. */
  waiting: boolean;
}

export interface AiUpdateKindStatus {
  kind: AiUpdateKind;
  policy: AiUpdatePolicy;
  /** Epoch ms. */
  lastCheck: number | null;
  lastSuccess: number | null;
  /** Why the last check or update failed (quiet: shown in Settings only). */
  lastError: string | null;
  /** Offer "Download manually" (certificate interception, proxy sign-in, blocked download). */
  manualSuggested: boolean;
  /** Epoch ms of the next automatic check; null with Manual. */
  nextCheck: number | null;
  checking: boolean;
  updates: AiUpdate[];
}

export interface AiUpdatesStatus {
  intervalHours: number;
  kinds: AiUpdateKindStatus[];
}
