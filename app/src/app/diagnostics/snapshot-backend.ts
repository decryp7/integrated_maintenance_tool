import { invoke } from '@tauri-apps/api/core';
import { EngineError, ENGINE_ERROR_EVENT, SnapshotContext, SnapshotInfo, SnapshotSaveResult } from '../core/api.types';
import { CapturedImage } from './snapshot.types';

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
  abstract exportDiagnostics(path: string, ids: string[]): Promise<number>;
  /** Whether automatic snapshots are on (the shared diagnostics setting). */
  abstract automaticEnabled(): Promise<boolean>;
  /** Calls `handler` for every `imt://error` event. */
  abstract onEngineError(handler: (error: EngineError) => void): Promise<Unlisten>;
}

export type ListenFn = (event: string, handler: (payload: unknown) => void) => Promise<Unlisten>;

/* v8 ignore start: thin wrapper over the Tauri event API, exercised only in the real app */
const tauriListen: ListenFn = async (event, handler) => {
  const { listen } = await import('@tauri-apps/api/event');
  return listen(event, (e) => handler(e.payload));
};
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
  exportDiagnostics(path: string, ids: string[]) {
    return this.call<number>('export_diagnostics', { path, ids });
  }
  async automaticEnabled() {
    const s = await this.call<{ snapshots: { enabled: boolean } }>('diagnostics_settings');
    return s.snapshots.enabled;
  }
  onEngineError(handler: (error: EngineError) => void) {
    return this.listen(ENGINE_ERROR_EVENT, (p) => handler(p as EngineError));
  }
}
