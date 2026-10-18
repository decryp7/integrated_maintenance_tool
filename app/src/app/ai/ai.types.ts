/**
 * Wire contract of the local AI assistant (`app/src-tauri/src/ai/**`, `crates/imt-ai`), camelCase JSON.
 * Commands: `ai_*` (see `ai-backend.ts`). Events: `imt://ai-setup` (`SetupEvent`, all windows),
 * `imt://ai-status` (`AiStatus`, all windows), `imt://ai-chat` (`ChatEventEnvelope`, the chat's window),
 * `imt://ai-runtime` (`AiRuntimeEvent`, all windows).
 * Every command rejects with `{ code, message }` (`AiErrorPayload`).
 */
import { ExplainNode, InferredGroup, ParsedPreview } from '../core/api.types';

export const AI_SETUP_EVENT = 'imt://ai-setup';
export const AI_STATUS_EVENT = 'imt://ai-status';
export const AI_CHAT_EVENT = 'imt://ai-chat';
export const AI_RUNTIME_EVENT = 'imt://ai-runtime';

export type AiErrorCode =
  | 'unavailable'
  | 'noModel'
  | 'noRuntime'
  | 'busy'
  | 'cancelled'
  | 'invalid'
  | 'engine'
  | 'contextOverflow'
  | 'malformed'
  | 'toolLoopLimit'
  | 'modelFile'
  | 'memory'
  | 'network'
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
