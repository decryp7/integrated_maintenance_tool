import { inject, Injectable, signal } from '@angular/core';
import { UiStateContribution } from '../lifecycle/ui-state';
import { DumpsStore } from './dumps.store';

/** Version of the saved dumps state; other versions are read field by field. */
export const DUMPS_STATE_VERSION = 1;
/** Key of the dumps state in the project's (and the autosaved session's) `ui` object. */
export const DUMPS_STATE_KEY = 'dumps';
/** Bound on restored documents (the engine keeps at most 32 open). */
const MAX_DOCS = 32;

/** What a project or session remembers: the open documents by path, the shown dump and the UMDH comparison. */
export interface DumpsUiState {
  version: number;
  paths: string[];
  dump: string | null;
  umdhTarget: string | null;
  umdhBase: string | null;
}

const str = (v: unknown): string | null => (typeof v === 'string' && v.length > 0 ? v : null);

/** Saved state (anything: missing, corrupt, older or newer) as a valid `DumpsUiState`; null when it is no object. */
export function parseDumpsUiState(value: unknown): DumpsUiState | null {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return null;
  const v = value as Record<string, unknown>;
  const paths = Array.isArray(v['paths']) ? [...new Set(v['paths'].map(str).filter((p): p is string => p !== null))].slice(0, MAX_DOCS) : [];
  return {
    version: typeof v['version'] === 'number' ? v['version'] : DUMPS_STATE_VERSION,
    paths,
    dump: str(v['dump']),
    umdhTarget: str(v['umdhTarget']),
    umdhBase: str(v['umdhBase']),
  };
}

/**
 * Saves and restores the open dumps and UMDH logs with the project and the session. Restoring reopens the files
 * through the engine; files that no longer exist are a notice in the views, not a failure.
 */
@Injectable({ providedIn: 'root' })
export class DumpsPersistence {
  private readonly store = inject(DumpsStore);
  /** The last restore, so tests (and callers) can wait for it. */
  pending: Promise<void> = Promise.resolve();
  /** What is being restored: reported by `capture` until done, so reopening does not mark the project changed. */
  private readonly restoring = signal<DumpsUiState | null>(null);

  capture(): DumpsUiState {
    const target = this.restoring();
    if (target) return target;
    const docs = this.store.docs();
    const pathOf = (id: number | null) => (id === null ? null : (docs.find((d) => d.id === id)?.path ?? null));
    return {
      version: DUMPS_STATE_VERSION,
      paths: docs.map((d) => d.path).filter((p): p is string => !!p),
      dump: pathOf(this.store.activeDump()),
      umdhTarget: pathOf(this.store.umdhTarget()),
      umdhBase: pathOf(this.store.umdhBase()),
    };
  }

  restore(saved: DumpsUiState | null): Promise<void> {
    this.restoring.set(saved ? { ...saved, version: DUMPS_STATE_VERSION } : { version: DUMPS_STATE_VERSION, paths: [], dump: null, umdhTarget: null, umdhBase: null });
    this.pending = this.apply(saved).finally(() => this.restoring.set(null));
    return this.pending;
  }

  private async apply(saved: DumpsUiState | null): Promise<void> {
    if (!saved) {
      if (this.store.docs().length) await this.store.closeAll();
      return;
    }
    const wanted = new Set(saved.paths);
    for (const d of this.store.docs()) if (!d.path || !wanted.has(d.path)) await this.store.close(d.id);
    const open = new Set(this.store.docs().map((d) => d.path));
    await this.store.restorePaths(saved.paths.filter((p) => !open.has(p)));
    const idOf = (path: string | null) => (path === null ? null : (this.store.docs().find((d) => d.path === path)?.id ?? null));
    const dump = idOf(saved.dump);
    if (dump !== null) this.store.setActiveDump(dump);
    const target = idOf(saved.umdhTarget);
    if (target !== null) this.store.setUmdh(target, idOf(saved.umdhBase));
  }

  /** The `ui` state contribution (registered by `provideDumps`). */
  readonly contribution: UiStateContribution = {
    key: DUMPS_STATE_KEY,
    capture: () => this.capture(),
    restore: (value) => void this.restore(parseDumpsUiState(value)),
    reset: () => void this.restore(null),
  };
}
