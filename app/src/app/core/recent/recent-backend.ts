import { inject, Injectable } from '@angular/core';
import { invoke } from '../activity/ipc';
import { isTauri } from '../platform';
import { browserMocks } from '../mock-registry';
import { Logger, withFailureLogging } from '../logger';
import { listenHere, PayloadListenFn, Unlisten } from '../../windowing/window-events';
import { RecentKind, RecentLists, RecentProject } from '../api.types';

/** Emitted to every window when a recent list changed (payload: `RecentLists`). */
export const RECENT_EVENT = 'imt://recent-changed';

/** The real backend inside Tauri, the in-memory mock in a plain browser; failures are logged. */
export function createRecentBackend(): RecentBackend {
  const real = isTauri() ? new TauriRecentBackend() : new (browserMocks().MockRecentBackend)();
  return withFailureLogging<RecentBackend>(real, inject(Logger));
}

/**
 * The recent files and projects. Rust owns them: it records an entry after a granted open or save, and
 * grants an entry again when it is reopened from the list. App-wide (every window shows the same lists).
 */
@Injectable({ providedIn: 'root', useFactory: createRecentBackend })
export abstract class RecentBackend {
  abstract list(): Promise<RecentLists>;
  abstract remove(kind: RecentKind, path: string): Promise<void>;
  abstract clear(kind: RecentKind): Promise<void>;
  /**
   * The UI opened or saved these. Rust has recorded them already (nothing to do there); the browser
   * mock, which has no engine behind it, records them here.
   */
  abstract opened(files: string[], project?: RecentProject, captures?: string[]): Promise<void>;
  abstract onChanged(handler: (lists: RecentLists) => void): Promise<Unlisten>;
}

/** Parses the lists from the wire (anything malformed reads as empty). */
export function parseRecentLists(value: unknown): RecentLists {
  const v = (value ?? {}) as { files?: unknown; projects?: unknown; captures?: unknown };
  const files = Array.isArray(v.files) ? v.files.filter((p): p is string => typeof p === 'string') : [];
  const captures = Array.isArray(v.captures) ? v.captures.filter((p): p is string => typeof p === 'string') : [];
  const projects = Array.isArray(v.projects)
    ? v.projects.filter((p): p is RecentProject => !!p && typeof (p as RecentProject).path === 'string' && typeof (p as RecentProject).name === 'string')
    : [];
  return { files, projects, captures };
}

/** Talks to the Rust shell. `call` and `listen` are injectable for tests. */
export class TauriRecentBackend extends RecentBackend {
  constructor(
    private readonly call: typeof invoke = invoke,
    private readonly listen: PayloadListenFn = listenHere,
  ) {
    super();
  }

  async list() {
    return parseRecentLists(await this.call<unknown>('recent_list'));
  }
  remove(kind: RecentKind, path: string) {
    return this.call<void>('recent_remove', { kind, path });
  }
  clear(kind: RecentKind) {
    return this.call<void>('recent_clear', { kind });
  }
  async opened() {
    // Rust recorded the open or save itself.
  }
  onChanged(handler: (lists: RecentLists) => void) {
    return this.listen(RECENT_EVENT, (p) => handler(parseRecentLists(p)));
  }
}
