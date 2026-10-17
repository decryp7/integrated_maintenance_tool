import { invoke } from '@tauri-apps/api/core';
import { inject, Provider } from '@angular/core';
import {
  ChartData,
  ChartRequest,
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
}

/** Talks to the Rust core through Tauri commands. */
export class TauriBackend extends ImtBackend {
  workspaceSummary() {
    return invoke<WorkspaceSummary>('workspace_summary');
  }
  openPaths(paths: string[]) {
    return invoke<WorkspaceSummary>('open_paths', { paths });
  }
  removeSource(sourceId: number) {
    return invoke<WorkspaceSummary>('remove_source', { sourceId });
  }
  clearWorkspace() {
    return invoke<WorkspaceSummary>('clear_workspace');
  }
  reloadWorkspace() {
    return invoke<WorkspaceSummary>('reload_workspace');
  }
  queryPage(request: PageRequest) {
    return invoke<Page>('query_page', { request });
  }
  queryTimeline(request: TimelineRequest) {
    return invoke<Timeline>('query_timeline', { request });
  }
  queryFacet(request: FacetRequest) {
    return invoke<Facet>('query_facet', { request });
  }
  queryChart(request: ChartRequest) {
    return invoke<ChartData>('query_chart', { request });
  }
  getProfile() {
    return invoke<ProfileDoc>('get_profile');
  }
  validateProfile(toml: string) {
    return invoke<ProfileValidation>('validate_profile', { toml });
  }
  saveProfile(toml: string) {
    return invoke<WorkspaceSummary>('save_profile', { toml });
  }
  previewExtractor(request: ExtractPreviewRequest) {
    return invoke<ExtractPreview>('preview_extractor', { request });
  }
  importProfile(path: string) {
    return invoke<ProfileDoc>('import_profile', { path });
  }
  exportProfile(path: string, toml: string) {
    return invoke<void>('export_profile', { path, toml });
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
