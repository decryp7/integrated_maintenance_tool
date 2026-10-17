import { invoke } from '@tauri-apps/api/core';
import { inject, Provider } from '@angular/core';
import {
  AlertTarget,
  AnnotationDto,
  AnnotationsRequest,
  ChartData,
  ChartRequest,
  ContextRequest,
  ExportRequest,
  ExportResult,
  ExplainNode,
  ExtractPreview,
  ExtractPreviewRequest,
  Facet,
  FacetRequest,
  InferredRegex,
  LIVE_ALERT_EVENT,
  LiveAlert,
  LiveOptions,
  LiveState,
  Page,
  PageRequest,
  ParsePreview,
  ParsePreviewRequest,
  ProfileDoc,
  ProfileValidation,
  SourceContext,
  SubscriberMetrics,
  TailFileChange,
  TailReport,
  TailStatus,
  Timeline,
  TimelineRequest,
  WorkspaceSummary,
} from './api.types';
import { reportBackendFailures } from './failure-reporter';
import { Logger, withFailureLogging } from './logger';
import { isTauri } from './platform';
import { MockBackend } from './mock/mock-backend';
import { mockOptionsFromSearch } from './mock/mock-options';
import { listenHere } from '../windowing/window-events';

/** Everything the UI can ask the backend. Injected as a class token. */
export abstract class ImtBackend {
  abstract workspaceSummary(): Promise<WorkspaceSummary>;
  abstract openPaths(paths: string[]): Promise<WorkspaceSummary>;
  abstract removeSource(sourceId: number): Promise<WorkspaceSummary>;
  abstract clearWorkspace(): Promise<WorkspaceSummary>;
  abstract reloadWorkspace(): Promise<WorkspaceSummary>;
  abstract queryPage(request: PageRequest): Promise<Page>;
  abstract queryTimeline(request: TimelineRequest): Promise<Timeline>;
  abstract queryFacet(request: FacetRequest): Promise<Facet>;
  abstract queryChart(request: ChartRequest): Promise<ChartData>;
  abstract getProfile(): Promise<ProfileDoc>;
  abstract validateProfile(toml: string): Promise<ProfileValidation>;
  abstract saveProfile(toml: string): Promise<WorkspaceSummary>;
  abstract previewExtractor(request: ExtractPreviewRequest): Promise<ExtractPreview>;
  abstract importProfile(path: string): Promise<ProfileDoc>;
  abstract exportProfile(path: string, toml: string): Promise<void>;
  abstract exportEvents(request: ExportRequest): Promise<ExportResult>;
  /** The original lines around an event. */
  abstract sourceContext(request: ContextRequest): Promise<SourceContext>;
  /** Decoded first bytes of a file in the workspace (opened or unrecognised); rejects for any other path. */
  abstract readFileHead(path: string, maxBytes?: number): Promise<string>;
  /** Parses `sample` with a candidate (unsaved) profile, in the engine. */
  abstract previewParse(request: ParsePreviewRequest): Promise<ParsePreview>;
  /** Turns live tail on (with `options`, engine defaults for anything absent) or off; resolves to the number of watched folders. */
  abstract setLive(enabled: boolean, options?: LiveOptions): Promise<number>;
  /**
   * Calls `handler` whenever live tail changed the workspace, with what changed (null when the event carried no
   * payload). Resolves to the unsubscribe function.
   */
  abstract onWorkspaceChanged(handler: (report: TailReport | null) => void): Promise<() => void>;
  /** Calls `handler` for each alert raised by the live detectors and alert rules. Resolves to the unsubscribe function. */
  abstract onLiveAlert(handler: (alert: LiveAlert) => void): Promise<() => void>;
  /** Per-file live tail state and totals. */
  abstract tailStatus(): Promise<TailStatus>;
  /** Whether this window's live tail runs, and the options in effect (to restore the live controls after a reload). */
  abstract liveState(): Promise<LiveState>;
  /** Mutes this window's live alerts of one kind or rule until `untilMs` (wall clock) in the engine; rejects for an invalid mute. */
  abstract muteAlerts(target: AlertTarget, untilMs: number): Promise<void>;
  /** Lifts a mute set with `muteAlerts`. */
  abstract unmuteAlerts(target: AlertTarget): Promise<void>;
  /** Opens files and folders; with `startAtEnd`, text files are tailed from their current end (only new lines). */
  abstract openPathsLive(paths: string[], startAtEnd: boolean): Promise<WorkspaceSummary>;
  /** Infers a regex with named, typed groups from example lines (rejects for no lines). */
  abstract inferRegex(lines: string[]): Promise<InferredRegex>;
  /** Explains a regex in plain English as an indented outline (rejects for an invalid regex). */
  abstract explainRegex(pattern: string): Promise<ExplainNode[]>;
  /** Spans and points found by plugin annotators (reboots, service uptime, ...), by start time. */
  abstract annotations(request: AnnotationsRequest): Promise<AnnotationDto[]>;
  /** Event-bus delivery counters of the plugins' subscribers (plugin health). */
  abstract pluginEvents(): Promise<SubscriberMetrics[]>;
}

/** Tauri event emitted by the engine after live tail re-imported changed files. */
export const WORKSPACE_CHANGED_EVENT = 'imt://workspace-changed';

/** Talks to the Rust core through Tauri commands. */
export class TauriBackend extends ImtBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly subscribe: (event: string, handler: (payload: unknown) => void) => Promise<() => void> = tauriListen,
  ) {
    super();
  }

  workspaceSummary() {
    return this.call<WorkspaceSummary>('workspace_summary');
  }
  openPaths(paths: string[]) {
    return this.call<WorkspaceSummary>('open_paths', { paths });
  }
  removeSource(sourceId: number) {
    return this.call<WorkspaceSummary>('remove_source', { sourceId });
  }
  clearWorkspace() {
    return this.call<WorkspaceSummary>('clear_workspace');
  }
  reloadWorkspace() {
    return this.call<WorkspaceSummary>('reload_workspace');
  }
  queryPage(request: PageRequest) {
    return this.call<Page>('query_page', { request });
  }
  queryTimeline(request: TimelineRequest) {
    return this.call<Timeline>('query_timeline', { request });
  }
  queryFacet(request: FacetRequest) {
    return this.call<Facet>('query_facet', { request });
  }
  queryChart(request: ChartRequest) {
    return this.call<ChartData>('query_chart', { request });
  }
  getProfile() {
    return this.call<ProfileDoc>('get_profile');
  }
  validateProfile(toml: string) {
    return this.call<ProfileValidation>('validate_profile', { toml });
  }
  saveProfile(toml: string) {
    return this.call<WorkspaceSummary>('save_profile', { toml });
  }
  previewExtractor(request: ExtractPreviewRequest) {
    return this.call<ExtractPreview>('preview_extractor', { request });
  }
  importProfile(path: string) {
    return this.call<ProfileDoc>('import_profile', { path });
  }
  exportProfile(path: string, toml: string) {
    return this.call<void>('export_profile', { path, toml });
  }
  exportEvents(request: ExportRequest) {
    return this.call<ExportResult>('export_events', { request });
  }
  sourceContext(request: ContextRequest) {
    return this.call<SourceContext>('source_context', { request });
  }
  readFileHead(path: string, maxBytes?: number) {
    return this.call<string>('read_file_head', { path, maxBytes });
  }
  previewParse(request: ParsePreviewRequest) {
    return this.call<ParsePreview>('preview_parse', { request });
  }
  setLive(enabled: boolean, options?: LiveOptions) {
    return this.call<number>('set_live', options ? { enabled, options } : { enabled });
  }
  onWorkspaceChanged(handler: (report: TailReport | null) => void) {
    return this.subscribe(WORKSPACE_CHANGED_EVENT, (payload) => handler(asTailReport(payload)));
  }
  onLiveAlert(handler: (alert: LiveAlert) => void) {
    return this.subscribe(LIVE_ALERT_EVENT, (payload) => {
      if (isLiveAlert(payload)) handler(payload);
    });
  }
  tailStatus() {
    return this.call<TailStatus>('tail_status');
  }
  liveState() {
    return this.call<LiveState>('live_state');
  }
  muteAlerts(target: AlertTarget, untilMs: number) {
    return this.call<void>('mute_alerts', { target, untilMs });
  }
  unmuteAlerts(target: AlertTarget) {
    return this.call<void>('unmute_alerts', { target });
  }
  openPathsLive(paths: string[], startAtEnd: boolean) {
    return this.call<WorkspaceSummary>('open_paths_live', { paths, startAtEnd });
  }
  inferRegex(lines: string[]) {
    return this.call<InferredRegex>('infer_regex', { lines });
  }
  explainRegex(pattern: string) {
    return this.call<ExplainNode[]>('explain_regex', { pattern });
  }
  annotations(request: AnnotationsRequest) {
    return this.call<AnnotationDto[]>('annotations', { request });
  }
  pluginEvents() {
    return this.call<SubscriberMetrics[]>('plugin_events');
  }
}

/** Subscribes to a Tauri event addressed to this window (the event API is loaded lazily). */
function tauriListen(event: string, handler: (payload: unknown) => void): Promise<() => void> {
  return listenHere(event, handler);
}

/** The event payload as a report, or null when it is missing or not shaped like one (never trust the wire blindly). */
export function asTailReport(payload: unknown): TailReport | null {
  if (!payload || typeof payload !== 'object') return null;
  const p = payload as Partial<TailReport>;
  if (typeof p.appended !== 'number') return null;
  const ids = (v: unknown) => (Array.isArray(v) ? v.filter((x): x is number => typeof x === 'number') : []);
  return {
    appended: p.appended,
    updated: typeof p.updated === 'number' ? p.updated : 0,
    sourcesChanged: ids(p.sourcesChanged),
    added: ids(p.added),
    rotated: Array.isArray(p.rotated) ? p.rotated : [],
    truncated: ids(p.truncated),
    deleted: ids(p.deleted),
    reimported: ids(p.reimported),
    evicted: typeof p.evicted === 'number' ? p.evicted : 0,
    idsChangedFrom: typeof p.idsChangedFrom === 'number' ? p.idsChangedFrom : null,
    newestTs: typeof p.newestTs === 'number' ? p.newestTs : null,
    instancesChanged: Array.isArray(p.instancesChanged) ? p.instancesChanged.filter((x): x is string => typeof x === 'string') : [],
    fileStates: Array.isArray(p.fileStates)
      ? p.fileStates.filter((c): c is TailFileChange => !!c && typeof c.sourceId === 'number' && !!c.state && typeof c.state.kind === 'string')
      : [],
    alertsRaised: typeof p.alertsRaised === 'number' ? p.alertsRaised : 0,
    elapsedMs: typeof p.elapsedMs === 'number' ? p.elapsedMs : 0,
  };
}

/** True for a payload shaped like a `LiveAlert`. */
export function isLiveAlert(payload: unknown): payload is LiveAlert {
  if (!payload || typeof payload !== 'object') return false;
  const a = payload as Partial<LiveAlert>;
  return typeof a.id === 'string' && typeof a.kind === 'string' && typeof a.message === 'string' && typeof a.from === 'number' && typeof a.to === 'number' && !!a.evidenceQuery && typeof a.evidenceQuery === 'object';
}

/**
 * Uses the Rust backend inside Tauri, and an in-memory demo dataset in a plain browser (ng serve).
 * Failed commands are logged (command name and redacted error) by a decorator and passed to the `FailureReporter`.
 */
export function provideBackend(): Provider {
  return {
    provide: ImtBackend,
    useFactory: () =>
      withFailureLogging<ImtBackend>(isTauri() ? new TauriBackend() : new MockBackend(mockOptionsFromSearch(window.location.search)), inject(Logger), {
        onFailure: reportBackendFailures(),
      }),
  };
}
