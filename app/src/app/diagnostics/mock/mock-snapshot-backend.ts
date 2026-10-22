import { FILE_EXISTS_SUFFIX, WorkspaceSummary } from '../../core/api.types';
import { SnapshotBackend, Unlisten } from '../snapshot-backend';
import { mockJobs, mockSteps } from '../../jobs/mock-jobs';
import { EngineError, ProblemReportPreview, SnapshotContext, SnapshotInfo, SnapshotSaveResult, WindowCapture } from '../../core/api.types';
import { activity } from '../../core/activity/runtime';
import { Breadcrumb } from '../../core/activity/types';
import { steps } from '../../core/activity/steps';
import { CapturedImage } from '../snapshot.types';
import { DiagnosticsReportState } from '../diagnostics-report.types';
import { SAMPLE_REPORT_IMAGE, sampleReportState } from './mock-diagnostics-report';

/** A 160 × 90 slate-grey PNG: what the mock "captures" (the real shell asks the platform webview). */
export const MOCK_WINDOW_PNG =
  'iVBORw0KGgoAAAANSUhEUgAAAKAAAABaCAIAAACwpMoFAAAAo0lEQVR42u3RMQ0AAAgEsffDwo5/X/iAJqfgmurR4WIBYAEWYAEWYAEWYMACLMACLMACLMCABViABViABViABRiwAAuwAAuwAAswYAEWYAEWYAEWYMACLMACLMACLMACDFiABViABViABRiwAAuwAAuwAAswYBcAC7AAC7AAC7AAAxZgARZgARZgAQYswAIswAIswAIswIAFWIAFWIAFWIB/AS+ulfdhMQahHwAAAABJRU5ErkJggg==';
const MOCK_WINDOW_PNG_BYTES = 220;

/**
 * In-memory snapshot store for browser development and UI tests. It keeps the images the UI sends, so the view
 * shows them. It is not the reference for behaviour (the Rust engine is).
 */
export class MockSnapshotBackend extends SnapshotBackend {
  private readonly items: { info: SnapshotInfo; url: string }[] = [];
  private listeners = new Set<(e: EngineError) => void>();
  private n = 0;
  /**
   * Settings ▸ "Take error snapshots automatically" (`?mockSnapshots=off` turns it off, for UI tests of error states
   * that are not about snapshots, so no snapshot toast covers what they check).
   */
  enabled = typeof window === 'undefined' || new URLSearchParams(window.location.search).get('mockSnapshots') !== 'off';
  /** Calls made by the UI, for assertions. */
  readonly exports: { path: string; ids: string[] }[] = [];
  /** Problem reports "saved" by the UI, for assertions. */
  readonly reports: { path: string; description: string }[] = [];
  folderOpened = 0;

  /** `crumbs`: the window's latest activity (the Rust shell reads its own ring; the mock asks the UI's). */
  constructor(
    private readonly crumbs: (n: number) => Breadcrumb[] = (n) => activity.breadcrumbs(n),
    /** The workspace as it is now, to find an opened diagnostics zip. */
    private readonly workspace: () => WorkspaceSummary | null = () => null,
  ) {
    super();
  }

  /** The browser has no native capture: a small placeholder picture stands in for the window (and costs nothing). */
  async captureWindow(): Promise<WindowCapture> {
    return { status: 'captured', image: MOCK_WINDOW_PNG, mime: 'image/png', bytes: MOCK_WINDOW_PNG_BYTES };
  }

  async save(image: CapturedImage, context: SnapshotContext): Promise<SnapshotSaveResult> {
    if (!this.enabled && !context.manual) throw new Error('error snapshots are turned off in the settings');
    const id = `2026${String(++this.n).padStart(4, '0')}T000000000-${String(this.n).padStart(8, '0')}`;
    const breadcrumbs = this.crumbs(30);
    const info: SnapshotInfo = {
      id,
      createdAt: new Date().toISOString(),
      reason: context.reason,
      message: context.message,
      stack: context.stack ?? null,
      view: context.view ?? null,
      manual: context.manual,
      appVersion: 'mock',
      os: 'browser',
      mime: image.mime,
      bytes: image.bytes,
      breadcrumbs,
      steps: breadcrumbs.length ? steps(breadcrumbs, `Error: ${context.message}`) : [],
    };
    this.items.unshift({ info, url: `data:${image.mime};base64,${image.base64}` });
    return { id, firstTime: this.n === 1 };
  }
  async list() {
    return this.items.map((i) => i.info);
  }
  async image(id: string) {
    const item = this.items.find((i) => i.info.id === id);
    if (!item) throw new Error(`no snapshot "${id}"`);
    return item.url;
  }
  async remove(id: string) {
    const at = this.items.findIndex((i) => i.info.id === id);
    if (at < 0) throw new Error(`no snapshot "${id}"`);
    this.items.splice(at, 1);
  }
  async openFolder() {
    this.folderOpened++;
    return '/mock/logs/snapshots';
  }
  async exportDiagnostics(path: string, ids: string[], overwrite = false) {
    if (!overwrite && this.exports.some((x) => x.path === path)) throw new Error(`${path}${FILE_EXISTS_SUFFIX}`);
    this.exports.push({ path, ids });
    return this.fileJob('Export diagnostics to', path, `${ids.length * 2 + 1} files zipped.`);
  }

  /** Like the engine: the zip is written by an export job, returned at once. */
  private fileJob(title: string, path: string, summary: string): number {
    const name = path.split(/[\\/]/).pop() ?? path;
    const job = mockJobs().start({ kind: 'export', title: `${title} ${name}`, link: { type: 'path', path }, appWide: true }, async (ctx) => {
      await mockSteps(ctx, 200, 4);
      ctx.message(summary);
      return { value: null, result: { type: 'file', path } };
    });
    job.done.catch(() => undefined);
    return job.id;
  }
  async automaticEnabled() {
    return this.enabled;
  }
  async onEngineError(handler: (error: EngineError) => void): Promise<Unlisten> {
    this.listeners.add(handler);
    return () => this.listeners.delete(handler);
  }
  async reportPreview(description: string): Promise<ProblemReportPreview> {
    const crumbs = this.crumbs(200);
    const text = description.trim();
    const entries = [
      { name: 'description.txt', bytes: text.length, summary: text ? `Your description (${text.length} characters)` : 'No description' },
      { name: 'steps-to-reproduce.txt', bytes: 400, summary: `${steps(crumbs).length} steps from your recent activity` },
      { name: 'breadcrumbs.json', bytes: 2048, summary: `${crumbs.length} recent operations (no file contents)` },
      { name: 'system-info.json', bytes: 1536, summary: 'OS, Debrief version and build, memory, plugins, settings without secrets' },
      { name: 'manifest.json', bytes: 900, summary: 'What the report contains' },
      { name: 'processes.json', bytes: 700, summary: 'AI engine: 2 runs, 1 restart, last exit code 3' },
      { name: 'session-recovery.json', bytes: 300, summary: 'Previous run: closed properly, no recoverable session' },
      { name: 'logs/children/ai-worker.log', bytes: 4096, summary: 'AI worker: 1 run, no restarts' },
      { name: 'logs/children/llama-server.log', bytes: 8192, summary: 'AI engine: 2 runs, 1 restart, last exit code 3' },
      { name: 'logs/activity.log', bytes: 4096, summary: `${crumbs.length} activity entries` },
      { name: 'logs/debrief.log', bytes: 65536, summary: "Debrief's own log (recent part)" },
      ...this.items.flatMap((i) => [
        { name: `snapshots/${i.info.id}.png`, bytes: i.info.bytes, summary: `Screenshot: ${i.info.reason}` },
        { name: `snapshots/${i.info.id}.json`, bytes: 1024, summary: `Error details and ${i.info.breadcrumbs.length} breadcrumbs` },
      ]),
    ];
    return {
      entries,
      steps: steps(crumbs),
      systemInfo: { appVersion: 'mock', commit: 'unknown', sessionId: 'mock-session', os: 'browser', plugins: [] },
    };
  }
  async saveReport(path: string, description: string, overwrite = false) {
    if (!overwrite && this.reports.some((r) => r.path === path)) throw new Error(`${path}${FILE_EXISTS_SUFFIX}`);
    this.reports.push({ path, description });
    const files = (await this.reportPreview(description)).entries.length;
    return this.fileJob('Save problem report', path, `${files} files zipped.`);
  }
  /** Times the activity log was opened (a browser has no second window). */
  activityLogOpened = 0;
  async openActivityLog() {
    this.activityLogOpened++;
    return 'mock-activity';
  }
  async diagnosticsReport(): Promise<DiagnosticsReportState> {
    return sampleReportState(this.workspace()?.sources ?? []);
  }
  async diagnosticsReportImage(id: string) {
    if (!sampleReportState(this.workspace()?.sources ?? []).report?.snapshots.some((s) => s.id === id)) throw new Error(`no snapshot "${id}" in the report`);
    return SAMPLE_REPORT_IMAGE;
  }
  /** Test helper: delivers an `debrief://error` event. */
  emit(error: EngineError): void {
    this.listeners.forEach((l) => l(error));
  }
}
