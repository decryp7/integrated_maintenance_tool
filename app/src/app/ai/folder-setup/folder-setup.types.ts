/**
 * Wire contract of "Set up analysis for this folder…" (camelCase JSON). Mirrors `crates/imt-core/src/survey/dto.rs`
 * and `imt_ai::assistant::FolderRefinement`; the Tauri commands are in `app/src-tauri/src/ai/folder_setup.rs`.
 */
import { Level } from '../../core/api.types';

export interface SurveyRequest {
  maxEventsPerGroup?: number | null;
  maxEvents?: number | null;
  maxMillis?: number | null;
}

export type SurveyStage = 'sampling' | 'analysing' | 'proposing' | 'validating' | 'done';

export interface SurveyProgress {
  stage: SurveyStage;
  done: number;
  total: number;
}

export interface SourceGroup {
  id: string;
  importer: string;
  name: string;
  files: number;
  events: number;
  sampled: number;
  from: number | null;
  to: number | null;
}

export interface UnrecognisedGroup {
  extension: string;
  fileCount: number;
  files: string[];
  reason: string;
  sampleFile: string | null;
  sampleLines: string[];
}

export type FieldKind = 'number' | 'bool' | 'text' | 'mixed';

export interface FieldSummary {
  name: string;
  kind: FieldKind;
  present: number;
  coverage: number;
  distinct: number;
  distinctCapped: boolean;
  top: { value: string; count: number }[];
  min: number | null;
  max: number | null;
}

export type KeyOrigin = { kind: 'field' } | { kind: 'message'; regex: string };

export interface InstanceCandidate {
  field: string;
  origin: KeyOrigin;
  lanes: number;
  coverage: number;
  score: number;
  examples: string[];
}

export type LifecycleRole = 'start' | 'stop' | 'crash';
export type TemplateKind = 'serilog' | 'mined';

export interface LifecycleCandidate {
  role: LifecycleRole;
  template: string;
  templateKind: TemplateKind;
  count: number;
  example: string;
  coveredBy: string | null;
}

export interface TemplateSummary {
  template: string;
  templateKind: TemplateKind;
  count: number;
  level: Level;
  share: number;
  example: string;
}

export type NoiseReason = { kind: 'keyword'; word: string } | { kind: 'periodic'; intervalMs: number };

export interface NoiseCandidate {
  template: string;
  templateKind: TemplateKind;
  count: number;
  share: number;
  level: Level;
  reason: NoiseReason;
  example: string;
}

export interface NumberCandidate {
  name: string;
  label: string;
  unit: string | null;
  numberKind: 'number' | 'duration' | 'bytes';
  count: number;
  min: number;
  max: number;
  example: string;
  regex: string;
}

export interface MarkerCoverage {
  kind: string;
  label: string;
  lifecycle: boolean;
  matches: number;
}

/** Facts about the opened files: what the proposals (heuristic and AI) are grounded in. */
export interface FolderSurvey {
  totalEvents: number;
  sampledEvents: number;
  truncated: boolean;
  elapsedMs: number;
  groups: SourceGroup[];
  unrecognised: UnrecognisedGroup[];
  fields: FieldSummary[];
  instanceCandidates: InstanceCandidate[];
  lifecycle: LifecycleCandidate[];
  errors: TemplateSummary[];
  noise: NoiseCandidate[];
  numbers: NumberCandidate[];
  markers: MarkerCoverage[];
}

export type ProposalGroup = 'formats' | 'instances' | 'markers' | 'extractors' | 'noise' | 'charts' | 'queries';
export type ProposalOrigin = 'heuristic' | 'ai';

/**
 * One typed change. The profile sections are passed through as the engine wrote them (profile-schema
 * keys, see `default_profile.toml`); the UI shows them and sends them back, and the engine re-validates.
 */
export type Fragment =
  | { kind: 'parseRegex'; regex: string }
  | { kind: 'instanceKey'; key: string[] }
  | { kind: 'extractor'; extractor: Record<string, unknown> }
  | { kind: 'marker'; marker: Record<string, unknown> }
  | { kind: 'span'; span: Record<string, unknown> }
  | { kind: 'pipeline'; step: Record<string, unknown> }
  | { kind: 'chart'; chart: Record<string, unknown> }
  | { kind: 'dashboard'; dashboard: Record<string, unknown> }
  | { kind: 'savedQuery'; name: string; ql: string };

export interface LanePreview {
  name: string;
  count: number;
  from: number | null;
  to: number | null;
}

export type ProposalPreview =
  | { kind: 'parse'; parsed: number; lines: number; withTimestamp: number; examples: string[] }
  | { kind: 'lanes'; lanes: LanePreview[]; totalLanes: number; unassigned: number }
  | { kind: 'matches'; matches: number; sampled: number; examples: string[] }
  | { kind: 'extracted'; matches: number; sampled: number; examples: Record<string, unknown>[] }
  | { kind: 'runs'; runs: number; closed: number; open: number; noStop: number; lanes: number }
  | { kind: 'pipeline'; sampled: number; kept: number; dropped: number; changed: number; examples: { before: string; after: string }[] }
  | { kind: 'chart'; series: string[]; points: number }
  | { kind: 'dashboard'; tiles: number }
  | { kind: 'query'; matches: number };

export interface SetupProposal {
  id: string;
  group: ProposalGroup;
  origin: ProposalOrigin;
  title: string;
  summary: string;
  fragments: Fragment[];
  /** Ids of proposals this one needs (selected together). */
  requires: string[];
  /** The fragments as profile TOML, for review. */
  toml: string;
  preview: ProposalPreview;
  recommended: boolean;
}

export interface SkippedProposal {
  id: string;
  group: ProposalGroup;
  origin: ProposalOrigin;
  title: string;
  reason: string;
}

export interface FolderSetup {
  survey: FolderSurvey;
  proposals: SetupProposal[];
  skipped: SkippedProposal[];
}

/** `folder_survey`: the setup, or a cancelled survey (a normal outcome). */
export type SurveyOutcome = { setup: FolderSetup } | { cancelled: true };

export interface FolderRefinement {
  proposals: SetupProposal[];
  skipped: SkippedProposal[];
  /** Use cases that produced nothing, one line each. */
  notes: string[];
}

export type DiffOp = 'same' | 'add' | 'remove';

export interface ComposedProfile {
  toml: string;
  diff: { op: DiffOp; text: string }[];
  added: number;
  removed: number;
  savedQueries: { name: string; ql: string }[];
}

export const PROPOSAL_GROUPS: readonly ProposalGroup[] = ['formats', 'instances', 'markers', 'extractors', 'noise', 'charts', 'queries'];
