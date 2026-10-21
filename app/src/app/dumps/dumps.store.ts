import { computed, inject, Injectable } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { DumpDocSummary } from '../core/api.types';
import { errorText } from '../analysis/async-resource';
import { CmdError } from '../core/cmd-error';
import { browserMocks } from '../core/mock-registry';
import { nativePick } from '../core/native-dialog';
import { isTauri } from '../core/platform';
import { plural } from '../i18n/locale';
import { DumpsBackend } from './dumps-backend';

/** What to open: crash/memory dumps or UMDH logs (the native dialog's title and filter). */
export type DumpFileKind = 'dump' | 'umdh';

/** Chooses files: the native dialog in Tauri (which grants them to this window), the demo files in a browser. */
@Injectable({ providedIn: 'root' })
export class DumpPicker {
  pick(kind: DumpFileKind): Promise<string[]> {
    /* v8 ignore start: the native dialog exists only inside Tauri */
    if (isTauri()) {
      return kind === 'dump'
        ? nativePick({ kind: 'openFiles', title: $localize`:@@dumps.pick.dump:Open dump`, filters: [{ name: $localize`:@@dumps.pick.dumpFilter:Dumps`, extensions: ['dmp', 'mdmp'] }] })
        : nativePick({ kind: 'openFiles', title: $localize`:@@dumps.pick.umdh:Open UMDH snapshots or diffs`, filters: [{ name: $localize`:@@dumps.pick.umdhFilter:UMDH logs`, extensions: ['txt', 'log'] }] });
    }
    /* v8 ignore stop */
    const mocks = browserMocks();
    return Promise.resolve(kind === 'dump' ? [...mocks.MOCK_DUMP_PATHS] : [...mocks.MOCK_UMDH_PATHS]);
  }
}

interface DumpsShape {
  docs: DumpDocSummary[];
  loading: boolean;
  /** Last failure of open/close/list, shown inline with Retry. */
  error: string | null;
  /** Paths whose open failed, for Retry. */
  failedPaths: string[];
  notice: string | null;
  /** The dump the Dump view shows. */
  activeDump: number | null;
  /** The UMDH view's comparison: the newer log and the older snapshot (none: a diff log, or everything alive). */
  umdhTarget: number | null;
  umdhBase: number | null;
  /** Bumped whenever the open documents change (the timeline's markers reload). */
  version: number;
}

const isDump = (d: DumpDocSummary) => d.kind === 'dump';
const isUmdh = (d: DumpDocSummary) => d.kind !== 'dump';

/** UMDH logs oldest first (by time, then as opened). */
export function bySnapshotTime(docs: readonly DumpDocSummary[]): DumpDocSummary[] {
  return [...docs].sort((a, b) => (a.timeMs ?? 0) - (b.timeMs ?? 0) || a.id - b.id);
}

/**
 * The dumps and UMDH logs open in this window (the engine keeps them per window). Opening, closing and listing go
 * through the backend; the store keeps which dump and which UMDH comparison the views show.
 */
export const DumpsStore = signalStore(
  { providedIn: 'root' },
  withState<DumpsShape>({ docs: [], loading: false, error: null, failedPaths: [], notice: null, activeDump: null, umdhTarget: null, umdhBase: null, version: 0 }),
  withComputed((s) => ({
    dumps: computed(() => s.docs().filter(isDump)),
    umdh: computed(() => bySnapshotTime(s.docs().filter(isUmdh))),
    dump: computed(() => s.docs().find((d) => d.id === s.activeDump()) ?? null),
  })),
  withMethods((s) => {
    const backend = inject(DumpsBackend);
    const picker = inject(DumpPicker);

    /** Keeps the active dump and the UMDH comparison valid after the list changed, preferring what was just opened. */
    const settle = (opened: DumpDocSummary[]) => {
      const docs = s.docs();
      const dumps = docs.filter(isDump);
      const newDump = opened.find(isDump);
      const active = newDump?.id ?? (dumps.some((d) => d.id === s.activeDump()) ? s.activeDump() : (dumps[0]?.id ?? null));
      const logs = bySnapshotTime(docs.filter(isUmdh));
      let target = logs.some((d) => d.id === s.umdhTarget()) ? s.umdhTarget() : null;
      let base = logs.some((d) => d.id === s.umdhBase()) ? s.umdhBase() : null;
      if (opened.some(isUmdh) || target === null) {
        // Newest log as the target; with snapshots, the oldest as the base.
        const last = logs[logs.length - 1] ?? null;
        target = last?.id ?? null;
        const snapshots = logs.filter((d) => d.kind === 'umdhSnapshot');
        base = last?.kind === 'umdhSnapshot' && snapshots.length > 1 ? snapshots[0]!.id : null;
      }
      if (base === target) base = null;
      patchState(s, { activeDump: active, umdhTarget: target, umdhBase: base, version: s.version() + 1 });
    };

    const refresh = async (): Promise<void> => {
      patchState(s, { loading: true });
      try {
        patchState(s, { docs: await backend.list(), error: null, failedPaths: [] });
        settle([]);
      } catch (e) {
        patchState(s, { error: errorText(e) });
      } finally {
        patchState(s, { loading: false });
      }
    };

    /** Opens each path (failures are collected so the rest still opens); missing files are a notice when restoring. */
    const openPaths = async (paths: string[], missingIsNotice = false): Promise<DumpDocSummary[]> => {
      if (!paths.length) return [];
      patchState(s, { loading: true, error: null, notice: null });
      const opened: DumpDocSummary[] = [];
      const errors: string[] = [];
      const failed: string[] = [];
      const missing: string[] = [];
      for (const path of paths) {
        try {
          opened.push(await backend.open(path));
        } catch (e) {
          if (missingIsNotice && e instanceof CmdError && e.code === 'notFound') {
            missing.push(path);
            continue;
          }
          errors.push(errorText(e));
          failed.push(path);
        }
      }
      try {
        patchState(s, { docs: await backend.list() });
      } catch (e) {
        errors.push(errorText(e));
      }
      settle(opened);
      patchState(s, { loading: false, error: errors.length ? errors.join('\n') : null, failedPaths: failed, notice: openNotice(opened.length, missing) });
      return opened;
    };

    return {
      refresh,
      openPaths,
      async pickAndOpen(kind: DumpFileKind): Promise<DumpDocSummary[]> {
        try {
          return await openPaths(await picker.pick(kind));
        } catch (e) {
          patchState(s, { error: errorText(e) });
          return [];
        }
      },
      /** Claims the dropped files the engine sniffs as dumps or UMDH logs and opens them; the rest go to the log import. */
      async openDropped(paths: string[]): Promise<{ claimed: string[]; opened: DumpDocSummary[] }> {
        if (!paths.length) return { claimed: [], opened: [] };
        const claimed = (await backend.sniff(paths)).filter((r) => r.kind !== 'none').map((r) => r.path);
        return { claimed, opened: await openPaths(claimed) };
      },
      restorePaths: (paths: string[]) => openPaths(paths, true),
      async close(id: number): Promise<void> {
        try {
          await backend.close(id);
          patchState(s, { docs: s.docs().filter((d) => d.id !== id), error: null });
          settle([]);
        } catch (e) {
          patchState(s, { error: errorText(e) });
        }
      },
      async closeAll(): Promise<void> {
        for (const d of s.docs()) {
          try {
            await backend.close(d.id);
          } catch {
            // Already closed: nothing to do.
          }
        }
        patchState(s, { docs: [], error: null, failedPaths: [], notice: null });
        settle([]);
      },
      retry(): Promise<unknown> {
        const failed = s.failedPaths();
        return failed.length ? openPaths(failed) : refresh();
      },
      dismiss: () => patchState(s, { error: null, notice: null, failedPaths: [] }),
      setActiveDump: (id: number | null) => patchState(s, { activeDump: id }),
      setUmdh(target: number | null, base: number | null): void {
        patchState(s, { umdhTarget: target, umdhBase: base === target ? null : base });
      },
    };
  }),
);

export type DumpsStoreType = InstanceType<typeof DumpsStore>;

/** "Opened 2 files." plus the files that could not be found; null when there is nothing to say. */
export function openNotice(opened: number, missing: readonly string[]): string | null {
  const parts: string[] = [];
  if (opened) {
    parts.push(plural(opened, { one: $localize`:@@dumps.opened.one:Opened ${opened}:count: file.`, other: $localize`:@@dumps.opened.other:Opened ${opened}:count: files.` }));
  }
  if (missing.length) {
    const names = missing.map((p) => p.split(/[\\/]/).pop() ?? p).join(', ');
    const n = missing.length;
    parts.push(
      plural(n, {
        one: $localize`:@@dumps.missing.one:${n}:count: file was not found: ${names}:names:.`,
        other: $localize`:@@dumps.missing.other:${n}:count: files were not found: ${names}:names:.`,
      }),
    );
  }
  return parts.length ? parts.join(' ') : null;
}
