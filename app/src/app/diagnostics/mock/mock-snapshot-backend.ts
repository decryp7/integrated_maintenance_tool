import { FILE_EXISTS_SUFFIX } from '../../core/api.types';
import { SnapshotBackend, Unlisten } from '../snapshot-backend';
import { EngineError, ProblemReportPreview, SnapshotContext, SnapshotInfo, SnapshotSaveResult } from '../../core/api.types';
import { activity } from '../../core/activity/runtime';
import { Breadcrumb } from '../../core/activity/types';
import { steps } from '../../core/activity/steps';
import { CapturedImage } from '../snapshot.types';

/**
 * In-memory snapshot store for browser development and UI tests. It keeps the images the UI really captured,
 * so the view shows what was rendered. It is not the reference for behaviour (the Rust engine is).
 */
export class MockSnapshotBackend extends SnapshotBackend {
  private readonly items: { info: SnapshotInfo; url: string }[] = [];
  private listeners = new Set<(e: EngineError) => void>();
  private n = 0;
  enabled = true;
  /** Calls made by the UI, for assertions. */
  readonly exports: { path: string; ids: string[] }[] = [];
  /** Problem reports "saved" by the UI, for assertions. */
  readonly reports: { path: string; description: string }[] = [];
  folderOpened = 0;

  /** `crumbs`: the window's latest activity (the Rust shell reads its own ring; the mock asks the UI's). */
  constructor(private readonly crumbs: (n: number) => Breadcrumb[] = (n) => activity.breadcrumbs(n)) {
    super();
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
    return ids.length * 2 + 1;
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
      { name: 'system-info.json', bytes: 1536, summary: 'OS, IMT version and build, memory, plugins, settings without secrets' },
      { name: 'logs/activity.log', bytes: 4096, summary: `${crumbs.length} activity entries` },
      { name: 'logs/imt.log', bytes: 65536, summary: "IMT's own log (recent part)" },
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
    return (await this.reportPreview(description)).entries.length;
  }
  /** Times the activity log was opened (a browser has no second window). */
  activityLogOpened = 0;
  async openActivityLog() {
    this.activityLogOpened++;
    return 'mock-activity';
  }
  /** Test helper: delivers an `imt://error` event. */
  emit(error: EngineError): void {
    this.listeners.forEach((l) => l(error));
  }
}
