import { invoke } from '@tauri-apps/api/core';
import { inject, Provider } from '@angular/core';
import {
  ChartData,
  ChartRequest,
  ExportRequest,
  ExportResult,
  ExtractPreview,
  ExtractPreviewRequest,
  Facet,
  FacetRequest,
  Page,
  PageRequest,
  ProfileDoc,
  ProfileValidation,
  Timeline,
  TimelineRequest,
  WorkspaceSummary,
} from './api.types';
import { Logger, withFailureLogging } from './logger';
import { isTauri } from './platform';
import { MockBackend } from './mock/mock-backend';

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
}

/** Talks to the Rust core through Tauri commands. */
export class TauriBackend extends ImtBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
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
}

/**
 * Uses the Rust backend inside Tauri, and an in-memory demo dataset in a plain browser (ng serve).
 * Failed commands are logged (command name and redacted error) by a decorator.
 */
export function provideBackend(): Provider {
  return {
    provide: ImtBackend,
    useFactory: () => withFailureLogging<ImtBackend>(isTauri() ? new TauriBackend() : new MockBackend(), inject(Logger)),
  };
}
