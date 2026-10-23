import { invoke } from '../core/activity/ipc';
import { jobValue } from '../jobs/job-value';
import {
  Analyzed,
  BundleOpened,
  Finding,
  Query,
  RedactedCopyChoices,
  RedactedCopyOptions,
  RedactedCopyPreview,
  RedactedCopyRequest,
  RedactedExport,
  RedactionPreview,
  RedactionPreviewRequest,
  RedactionSummary,
  RedactionSummaryRequest,
  ReportExportRequest,
  TileData,
  TileRequest,
  VisualRequest,
  VisualSaved,
  WorkspaceSummary,
} from '../core/api.types';

/**
 * Everything the "explain and share" features ask the engine: the redacted view, redaction previews,
 * dashboard tiles, report and image exports, opening old `.debriefz` share bundles (sharing is saving a project now). Injected as a class token (see
 * `provideShare()`); the Tauri implementation calls the shell, the mock serves browser development.
 */
export abstract class ShareBackend {
  /** Switches this window's redacted view on or off (rejects when the profile has no redaction rules; switching it on
   * is `unsupported` with the current log store). */
  abstract setRedactedView(enabled: boolean): Promise<WorkspaceSummary>;
  /** A sample redacted with the active profile's `[redact]` rules, or with those of `toml` (unsaved). */
  abstract previewRedaction(request: RedactionPreviewRequest): Promise<RedactionPreview>;
  /** What redaction would change in the events of a query (`unsupported` with the current log store). */
  abstract redactionSummary(request: RedactionSummaryRequest): Promise<RedactionSummary>;
  /** One dashboard tile evaluated over the window's query. */
  abstract dashboardTile(request: TileRequest): Promise<TileData>;
  /** Writes a report as self-contained HTML or PDF as a background export job; resolves with the job at once. */
  abstract exportReport(request: ReportExportRequest): Promise<number>;
  /** Saves a chart or timeline image (PNG data URL or SVG markup) to a path chosen in the save dialog. */
  abstract saveVisual(request: VisualRequest): Promise<VisualSaved>;
  /** Opens an old share bundle (`.debriefz`) read-only (extracted to a temporary folder). */
  abstract openBundle(path: string): Promise<BundleOpened>;
  /** The automatic health check over the events of a query, most severe first (report findings), with its coverage. */
  abstract overview(query: Query): Promise<Analyzed<Finding[]>>;
  /** What a redacted copy can be made of: all detectors, the profile's custom rules and what the profile selects. */
  abstract redactedCopyChoices(): Promise<RedactedCopyChoices>;
  /** What the copy would change and leave out (rejects `invalid`, field "options", when nothing is picked). */
  abstract redactedCopyPreview(options: RedactedCopyOptions): Promise<RedactedCopyPreview>;
  /** Writes the redacted copy as a project file (`.debriefz`); never replaces a file unless `overwrite`. */
  abstract exportRedactedCopy(request: RedactedCopyRequest): Promise<RedactedExport>;
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
    return this.call<number>('export_report', { request });
  }
  saveVisual(request: VisualRequest) {
    return this.call<VisualSaved>('save_visual', { request });
  }
  /** The command starts an import job and returns its id; the opened bundle is the job's value. */
  async openBundle(path: string) {
    const job = await this.call<number>('open_bundle', { path });
    return jobValue<BundleOpened>(this.call, job);
  }
  overview(query: Query) {
    return this.call<Analyzed<Finding[]>>('overview', { request: { query } });
  }
  redactedCopyChoices() {
    return this.call<RedactedCopyChoices>('redacted_copy_choices');
  }
  redactedCopyPreview(options: RedactedCopyOptions) {
    return this.call<RedactedCopyPreview>('redacted_copy_preview', { options });
  }
  /** The command starts a file job and returns its id; what the copy holds is the job's value. */
  async exportRedactedCopy(request: RedactedCopyRequest) {
    const { path, name, options, ui, reports, overwrite } = request;
    const job = await this.call<number>('export_redacted_copy', {
      path,
      name,
      options,
      ui,
      ...(reports ? { reports } : {}),
      ...(overwrite ? { overwrite } : {}),
    });
    return jobValue<RedactedExport>(this.call, job);
  }
}
