/**
 * Wire types of an opened IMT diagnostics report (the zip "Report a problem" and `imt-app --export-diagnostics`
 * write). Mirrors `app/src-tauri/src/activity/report/open.rs` (camelCase JSON); re-exported by `core/api.types.ts`.
 *
 *   diagnostics_report       {}      -> DiagnosticsReportState (the calling window's report, if it opened one)
 *   diagnostics_report_image { id }  -> string (`data:` URL of one error snapshot of that report)
 */
import type { SnapshotBreadcrumb } from '../core/api.types';

/** What each file of a report is, as listed in its `manifest.json`. Unknown kinds are ignored. */
export type ReportFileKind =
  | 'log'
  | 'child-log'
  | 'activity'
  | 'system-info'
  | 'processes'
  | 'steps'
  | 'breadcrumbs'
  | 'snapshot'
  | 'snapshot-details'
  | 'description'
  | 'session-recovery';

/** One file listed in the manifest. */
export interface ReportManifestFile {
  /** Path inside the zip, e.g. `logs/children/ai-worker.log`. */
  name: string;
  kind: ReportFileKind;
  bytes: number;
}

/** The report's `manifest.json` (kind `imt-diagnostics`). */
export interface ReportManifest {
  schemaVersion: number;
  imtVersion: string;
  commit: string;
  /** RFC 3339, UTC. */
  createdAt: string;
  files: ReportManifestFile[];
}

/** Kind of a child process IMT supervises. */
export type ChildKind = 'ai-worker' | 'llama-server' | 'plugin-host' | 'plugin' | 'store-worker';

/** `running`: alive when the report was made; `unknown`: the log ended without an exit line (IMT itself stopped). */
export type ChildRunState = 'running' | 'exited' | 'stopped' | 'crashed' | 'quarantined' | 'unknown';

/** One child process in `processes.json`. */
export interface ProcessSummary {
  /** Log name, e.g. `ai-worker`, `plugin-net.decryptology.web`. */
  name: string;
  kind: ChildKind;
  version: string | null;
  /** Of the newest run. */
  pid: number | null;
  state: ChildRunState;
  /** Starts in the newest IMT session (the first is not a restart). */
  starts: number;
  restarts: number;
  /** "exit code 3", "signal 11", "stopped by IMT". */
  lastExit: string | null;
  /** RFC 3339, UTC. */
  lastExitAt: string | null;
  quarantined: boolean;
  quarantineReason: string | null;
}

/** One error snapshot in the report (its image is fetched with `diagnostics_report_image`). */
export interface ReportSnapshot {
  id: string;
  createdAt: string;
  reason: string;
  message: string;
  view: string | null;
  mime: string;
  bytes: number;
  steps: string[];
  breadcrumbs: SnapshotBreadcrumb[];
}

/** An opened diagnostics report, read-only. */
export interface DiagnosticsReportView {
  /** The zip's file name. */
  name: string;
  manifest: ReportManifest;
  /** OS, IMT version and build, memory, plugins, settings without secrets. */
  systemInfo: Record<string, unknown>;
  processes: ProcessSummary[];
  steps: string[];
  breadcrumbs: SnapshotBreadcrumb[];
  snapshots: ReportSnapshot[];
  /** The previous run's crash marker and recoverable sessions, when the report has them. */
  sessionRecovery: Record<string, unknown> | null;
  /** The user's description, when there is one. */
  description: string | null;
  /** Files the manifest lists that could not be read (missing, too large, malformed). */
  unreadable: string[];
}

/** What `diagnostics_report` returns. */
export interface DiagnosticsReportState {
  report: DiagnosticsReportView | null;
  /**
   * Set once after a zip that looked like a diagnostics report could not be read as one (malformed, too large, or a
   * newer format) and was opened as a plain archive instead.
   */
  notice: string | null;
}
