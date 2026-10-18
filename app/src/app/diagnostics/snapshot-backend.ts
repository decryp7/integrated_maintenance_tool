import { invoke } from '../core/activity/ipc';
import { DiagnosticsReportState } from './diagnostics-report.types';
import { EngineError, ENGINE_ERROR_EVENT, ProblemReportPreview, SnapshotContext, SnapshotInfo, SnapshotSaveResult } from '../core/api.types';
import { CapturedImage } from './snapshot.types';
import { listenHere } from '../windowing/window-events';

/** Stops listening to an event. */
export type Unlisten = () => void;

/** Everything the error snapshot feature asks of the host. Injected as a class token. */
export abstract class SnapshotBackend {
  abstract save(image: CapturedImage, context: SnapshotContext): Promise<SnapshotSaveResult>;
  /** Newest first. */
  abstract list(): Promise<SnapshotInfo[]>;
  /** The image as a `data:` URL. */
  abstract image(id: string): Promise<string>;
  abstract remove(id: string): Promise<void>;
  /** Shows the folder in the OS file manager; resolves to its path. */
  abstract openFolder(): Promise<string>;
  /** Zips the chosen snapshots and the recent IMT log to `path`; resolves to the number of files. */
  /** Zips the snapshots; never replaces an existing file unless `overwrite`. */
  abstract exportDiagnostics(path: string, ids: string[], overwrite?: boolean): Promise<number>;
  /** Whether automatic snapshots are on (the shared diagnostics setting). */
  abstract automaticEnabled(): Promise<boolean>;
  /** Calls `handler` for every `imt://error` event. */
  abstract onEngineError(handler: (error: EngineError) => void): Promise<Unlisten>;
  /** What a problem report would contain, for the user to review before saving. */
  abstract reportPreview(description: string): Promise<ProblemReportPreview>;
  /** Writes the problem report zip to `path`; never replaces an existing file unless `overwrite`. Resolves to the number of files. */
  abstract saveReport(path: string, description: string, overwrite?: boolean): Promise<number>;
  /** Opens IMT's own logs in a new window, filtered to the activity entries (Help ▸ Show activity log). Resolves to the window label. */
  abstract openActivityLog(): Promise<string>;
  /** The diagnostics report this window opened (a zip saved by "Report a problem"), if any. */
  abstract diagnosticsReport(): Promise<DiagnosticsReportState>;
  /** One error snapshot of the opened report, as a `data:` URL. */
  abstract diagnosticsReportImage(id: string): Promise<string>;
}

export type ListenFn = (event: string, handler: (payload: unknown) => void) => Promise<Unlisten>;

/* v8 ignore start: thin wrapper over the Tauri event API, exercised only in the real app */
const tauriListen: ListenFn = (event, handler) => listenHere(event, handler);
/* v8 ignore stop */

/** Talks to the Rust shell (`snapshots.rs`). `call` and `listen` are injectable for tests. */
export class TauriSnapshotBackend extends SnapshotBackend {
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly listen: ListenFn = tauriListen,
  ) {
    super();
  }
  save(image: CapturedImage, context: SnapshotContext) {
    return this.call<SnapshotSaveResult>('save_error_snapshot', { image: image.base64, mime: image.mime, context });
  }
  list() {
    return this.call<SnapshotInfo[]>('list_error_snapshots');
  }
  image(id: string) {
    return this.call<string>('read_error_snapshot', { id });
  }
  remove(id: string) {
    return this.call<void>('delete_error_snapshot', { id });
  }
  openFolder() {
    return this.call<string>('open_snapshots_folder');
  }
  exportDiagnostics(path: string, ids: string[], overwrite = false) {
    return this.call<number>('export_diagnostics', overwrite ? { path, ids, overwrite } : { path, ids });
  }
  async automaticEnabled() {
    const s = await this.call<{ snapshots: { enabled: boolean } }>('diagnostics_settings');
    return s.snapshots.enabled;
  }
  onEngineError(handler: (error: EngineError) => void) {
    return this.listen(ENGINE_ERROR_EVENT, (p) => handler(p as EngineError));
  }
  reportPreview(description: string) {
    return this.call<ProblemReportPreview>('problem_report_preview', { description });
  }
  saveReport(path: string, description: string, overwrite = false) {
    return this.call<number>('save_problem_report', overwrite ? { path, description, overwrite } : { path, description });
  }
  openActivityLog() {
    return this.call<string>('open_activity_log');
  }
  diagnosticsReport() {
    return this.call<DiagnosticsReportState>('diagnostics_report', {});
  }
  diagnosticsReportImage(id: string) {
    return this.call<string>('diagnostics_report_image', { id });
  }
}
