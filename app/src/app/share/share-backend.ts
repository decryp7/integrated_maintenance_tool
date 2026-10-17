import { invoke } from '../core/activity/ipc';
import {
  BundleEstimate,
  BundleManifest,
  BundleOpened,
  BundleOptions,
  Finding,
  Query,
  RedactionPreview,
  RedactionPreviewRequest,
  RedactionSummary,
  RedactionSummaryRequest,
  ReportExported,
  ReportExportRequest,
  TileData,
  TileRequest,
  VisualRequest,
  VisualSaved,
  WorkspaceSummary,
} from '../core/api.types';

/**
 * Everything the "explain and share" features ask the engine: the redacted view, redaction previews,
 * dashboard tiles, report and image exports, and `.imtz` bundles. Injected as a class token (see
 * `provideShare()`); the Tauri implementation calls the shell, the mock serves browser development.
 */
export abstract class ShareBackend {
  /** Switches this window's redacted view on or off (rejects when the profile has no redaction rules). */
  abstract setRedactedView(enabled: boolean): Promise<WorkspaceSummary>;
  /** A sample redacted with the active profile's `[redact]` rules, or with those of `toml` (unsaved). */
  abstract previewRedaction(request: RedactionPreviewRequest): Promise<RedactionPreview>;
  /** What redaction would change in the events of a query. */
  abstract redactionSummary(request: RedactionSummaryRequest): Promise<RedactionSummary>;
  /** One dashboard tile evaluated over the window's query. */
  abstract dashboardTile(request: TileRequest): Promise<TileData>;
  /** Writes a report as self-contained HTML or PDF. */
  abstract exportReport(request: ReportExportRequest): Promise<ReportExported>;
  /** Saves a chart or timeline image (PNG data URL or SVG markup) to a path chosen in the save dialog. */
  abstract saveVisual(request: VisualRequest): Promise<VisualSaved>;
  /** What a bundle with `options` would contain. */
  abstract estimateBundle(options: BundleOptions): Promise<BundleEstimate>;
  /** Writes a shareable `.imtz` bundle; never replaces an existing file unless `overwrite`. */
  abstract createBundle(options: BundleOptions, path: string, overwrite?: boolean): Promise<BundleManifest>;
  /** Opens a `.imtz` read-only (extracted to a temporary folder). */
  abstract openBundle(path: string): Promise<BundleOpened>;
  /** The automatic health check over the events of a query, most severe first (report findings). */
  abstract overview(query: Query): Promise<Finding[]>;
}

/** Talks to the Rust shell (app/src-tauri/src/share.rs). */
export class TauriShareBackend extends ShareBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }

  setRedactedView(enabled: boolean) {
    return this.call<WorkspaceSummary>('set_redacted_view', { enabled });
  }
  previewRedaction(request: RedactionPreviewRequest) {
    return this.call<RedactionPreview>('preview_redaction', { request });
  }
  redactionSummary(request: RedactionSummaryRequest) {
    return this.call<RedactionSummary>('redaction_summary', { request });
  }
  dashboardTile(request: TileRequest) {
    return this.call<TileData>('dashboard_tile', { request });
  }
  exportReport(request: ReportExportRequest) {
    return this.call<ReportExported>('export_report', { request });
  }
  saveVisual(request: VisualRequest) {
    return this.call<VisualSaved>('save_visual', { request });
  }
  estimateBundle(options: BundleOptions) {
    return this.call<BundleEstimate>('estimate_bundle', { options });
  }
  createBundle(options: BundleOptions, path: string, overwrite = false) {
    return this.call<BundleManifest>('create_bundle', overwrite ? { options, path, overwrite } : { options, path });
  }
  openBundle(path: string) {
    return this.call<BundleOpened>('open_bundle', { path });
  }
  overview(query: Query) {
    return this.call<Finding[]>('overview', { request: { query } });
  }
}
