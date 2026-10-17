import { invoke } from '@tauri-apps/api/core';
import { inject, Provider } from '@angular/core';
import {
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
  Page,
  PageRequest,
  ParsePreview,
  ParsePreviewRequest,
  ProfileDoc,
  ProfileValidation,
  SourceContext,
  Timeline,
  TimelineRequest,
  WorkspaceSummary,
} from './api.types';
import { reportBackendFailures } from './failure-reporter';
import { Logger, withFailureLogging } from './logger';
import { isTauri } from './platform';
import { MockBackend } from './mock/mock-backend';
import { mockOptionsFromSearch } from './mock/mock-options';

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
  /** Turns live tail on or off; resolves to the number of watched paths. */
  abstract setLive(enabled: boolean): Promise<number>;
  /** Calls `handler` whenever live tail changed the workspace. Resolves to the unsubscribe function. */
  abstract onWorkspaceChanged(handler: () => void): Promise<() => void>;
  /** Infers a regex with named, typed groups from example lines (rejects for no lines). */
  abstract inferRegex(lines: string[]): Promise<InferredRegex>;
  /** Explains a regex in plain English as an indented outline (rejects for an invalid regex). */
  abstract explainRegex(pattern: string): Promise<ExplainNode[]>;
}

/** Tauri event emitted by the engine after live tail re-imported changed files. */
export const WORKSPACE_CHANGED_EVENT = 'imt://workspace-changed';

/** Talks to the Rust core through Tauri commands. */
export class TauriBackend extends ImtBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly subscribe: (event: string, handler: () => void) => Promise<() => void> = tauriListen,
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
  setLive(enabled: boolean) {
    return this.call<number>('set_live', { enabled });
  }
  onWorkspaceChanged(handler: () => void) {
    return this.subscribe(WORKSPACE_CHANGED_EVENT, handler);
  }
  inferRegex(lines: string[]) {
    return this.call<InferredRegex>('infer_regex', { lines });
  }
  explainRegex(pattern: string) {
    return this.call<ExplainNode[]>('explain_regex', { pattern });
  }
}

/** Subscribes to a Tauri event (loaded lazily so the browser build does not need the event API). */
async function tauriListen(event: string, handler: () => void): Promise<() => void> {
  const { listen } = await import('@tauri-apps/api/event');
  return listen(event, () => handler());
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
