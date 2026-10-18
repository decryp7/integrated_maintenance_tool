import { inject } from '@angular/core';
import { invoke } from '../core/activity/ipc';
import { Logger, withFailureLogging } from '../core/logger';
import { isTauri } from '../core/platform';
import { listenHere, PayloadListenFn, Unlisten } from '../windowing/window-events';
import { MockUpdatesBackend } from './mock-updates-backend';
import { UPDATES_EVENT, UpdateStatus } from './updates.types';

/** The updater in the Rust shell (`updater.rs`). Injected as a class token. */
export abstract class UpdatesBackend {
  abstract status(): Promise<UpdateStatus>;
  abstract check(): Promise<UpdateStatus>;
  /** Starts the download; progress arrives through `onStatus`. */
  abstract download(): Promise<UpdateStatus>;
  abstract cancel(): Promise<UpdateStatus>;
  abstract installOnQuit(enabled: boolean): Promise<UpdateStatus>;
  /** IMT quits, installs the verified update and starts again. */
  abstract restart(): Promise<void>;
  abstract openReleases(): Promise<void>;
  /** Every status change (in any window). */
  abstract onStatus(handler: (status: UpdateStatus) => void): Promise<Unlisten>;
}

/* v8 ignore start: thin wrapper over the Tauri event API, exercised only in the real app */
const tauriListen: PayloadListenFn = (event, handler) => listenHere(event, handler);
/* v8 ignore stop */

export class TauriUpdatesBackend extends UpdatesBackend {
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly listen: PayloadListenFn = tauriListen,
  ) {
    super();
  }
  status() {
    return this.call<UpdateStatus>('updates_status');
  }
  check() {
    return this.call<UpdateStatus>('updates_check');
  }
  download() {
    return this.call<UpdateStatus>('updates_download');
  }
  cancel() {
    return this.call<UpdateStatus>('updates_cancel');
  }
  installOnQuit(enabled: boolean) {
    return this.call<UpdateStatus>('updates_install_on_quit', { enabled });
  }
  restart() {
    return this.call<void>('updates_restart');
  }
  openReleases() {
    return this.call<void>('updates_open_releases');
  }
  onStatus(handler: (status: UpdateStatus) => void) {
    return this.listen(UPDATES_EVENT, (p) => handler(p as UpdateStatus));
  }
}

/** The Rust updater inside Tauri; a simulated one in a plain browser (ng serve, e2e). */
export function createUpdatesBackend(): UpdatesBackend {
  const backend = isTauri() ? new TauriUpdatesBackend() : new MockUpdatesBackend(typeof location === 'undefined' ? '' : location.search);
  return withFailureLogging<UpdatesBackend>(backend, inject(Logger), { category: 'ui.updates' });
}
