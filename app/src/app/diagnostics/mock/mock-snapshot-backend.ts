import { SnapshotBackend, Unlisten } from '../snapshot-backend';
import { EngineError, SnapshotContext, SnapshotInfo, SnapshotSaveResult } from '../../core/api.types';
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
  folderOpened = 0;

  async save(image: CapturedImage, context: SnapshotContext): Promise<SnapshotSaveResult> {
    if (!this.enabled && !context.manual) throw new Error('error snapshots are turned off in the settings');
    const id = `2026${String(++this.n).padStart(4, '0')}T000000000-${String(this.n).padStart(8, '0')}`;
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
  async exportDiagnostics(path: string, ids: string[]) {
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
  /** Test helper: delivers an `imt://error` event. */
  emit(error: EngineError): void {
    this.listeners.forEach((l) => l(error));
  }
}
