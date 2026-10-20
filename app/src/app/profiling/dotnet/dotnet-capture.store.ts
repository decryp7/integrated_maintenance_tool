import { computed, inject, Injectable, Injector } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { DotnetCaptureKind, DotnetCaptureStatus, DotnetProcess } from '../../core/api.types';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { CmdError } from '../../core/cmd-error';
import { errorMessage } from '../../tools/tool-errors';
import { CapturesStore } from '../captures.store';
import { showOpened } from '../provide-profiling';
import { DotnetBackend } from './dotnet-backend';

/** How often running captures are polled. */
export const POLL_MS = 1000;

/** Opens a finished capture in the profiling views. Replaceable in tests. */
@Injectable({ providedIn: 'root' })
export class DotnetOpener {
  private readonly captures = inject(CapturesStore);
  private readonly injector = inject(Injector);

  async open(path: string): Promise<void> {
    const opened = await this.captures.openPaths([path]);
    showOpened(this.injector.get(COMMAND_CONTEXT), opened);
  }
}

/** Whether a capture still runs. */
export function isRunning(c: DotnetCaptureStatus): boolean {
  return c.state === 'recording' || c.state === 'stopping';
}

/** Case-insensitive match on name, pid or command line. */
export function matchesProcess(p: DotnetProcess, filter: string): boolean {
  const f = filter.trim().toLowerCase();
  return !f || p.name.toLowerCase().includes(f) || String(p.pid).includes(f) || (p.commandLine ?? '').toLowerCase().includes(f);
}

interface DotnetState {
  processes: DotnetProcess[];
  loading: boolean;
  error: string | null;
  /** The error's `CmdError` code (`denied`: offer the restart as administrator). */
  errorCode: string | null;
  filter: string;
  selectedPid: number | null;
  kind: DotnetCaptureKind;
  /** null: until stopped. */
  durationSec: number | null;
  captures: DotnetCaptureStatus[];
  /** Captures already opened (each opens once). */
  opened: string[];
  busy: boolean;
}

const initial: DotnetState = {
  processes: [],
  loading: false,
  error: null,
  errorCode: null,
  filter: '',
  selectedPid: null,
  kind: 'cpu',
  durationSec: null,
  captures: [],
  opened: [],
  busy: false,
};

/** "Profile .NET process…": the process list, the capture choice and the running/finished captures. */
export const DotnetCaptureStore = signalStore(
  { providedIn: 'root' },
  withState(initial),
  withComputed((s) => ({
    visible: computed(() => s.processes().filter((p) => matchesProcess(p, s.filter()))),
    selected: computed(() => s.processes().find((p) => p.pid === s.selectedPid()) ?? null),
    running: computed(() => s.captures().filter(isRunning)),
  })),
  withComputed((s) => ({
    canRecord: computed(() => !!s.selected() && !s.busy() && !s.running().some((c) => c.pid === s.selectedPid())),
  })),
  withMethods((s, backend = inject(DotnetBackend), opener = inject(DotnetOpener)) => {
    const fail = (e: unknown) => patchState(s, { error: errorMessage(e), errorCode: e instanceof CmdError ? e.code : null });
    let timer: ReturnType<typeof setTimeout> | null = null;

    const openFinished = async (list: DotnetCaptureStatus[]) => {
      for (const c of list) {
        if (c.state === 'done' && c.path && !s.opened().includes(c.id)) {
          patchState(s, { opened: [...s.opened(), c.id] });
          try {
            await opener.open(c.path);
          } catch (e) {
            fail(e);
          }
        }
      }
    };

    const methods = {
      async refresh(): Promise<void> {
        patchState(s, { loading: true, error: null, errorCode: null });
        try {
          const processes = await backend.processes();
          const keep = processes.some((p) => p.pid === s.selectedPid()) ? s.selectedPid() : (processes[0]?.pid ?? null);
          patchState(s, { processes, selectedPid: keep });
          await methods.poll();
        } catch (e) {
          fail(e);
        } finally {
          patchState(s, { loading: false });
        }
      },
      setFilter(filter: string): void {
        patchState(s, { filter });
      },
      select(pid: number): void {
        patchState(s, { selectedPid: pid });
      },
      setKind(kind: DotnetCaptureKind): void {
        patchState(s, { kind });
      },
      setDuration(durationSec: number | null): void {
        const d = durationSec === null || !Number.isFinite(durationSec) ? null : Math.min(3600, Math.max(1, Math.round(durationSec)));
        patchState(s, { durationSec: d });
      },
      dismissError(): void {
        patchState(s, { error: null, errorCode: null });
      },
      /** Reads the captures; opens those that just finished; keeps polling while any runs. */
      async poll(): Promise<void> {
        try {
          const captures = await backend.captures();
          // Captures that existed before this window opened are not opened again.
          if (!s.captures().length && !s.opened().length) {
            patchState(s, { opened: captures.filter((c) => !isRunning(c)).map((c) => c.id) });
          }
          patchState(s, { captures });
          await openFinished(captures);
        } catch (e) {
          fail(e);
        }
        methods.schedule();
      },
      schedule(): void {
        if (timer !== null) return;
        if (!s.running().length) return;
        timer = setTimeout(() => {
          timer = null;
          void methods.poll();
        }, POLL_MS);
      },
      stopPolling(): void {
        if (timer !== null) clearTimeout(timer);
        timer = null;
      },
      async start(): Promise<DotnetCaptureStatus | null> {
        const p = s.selected();
        if (!p || !s.canRecord()) return null;
        patchState(s, { busy: true, error: null, errorCode: null });
        try {
          const durationSec = s.kind() === 'heap' ? null : s.durationSec();
          const status = await backend.start({ pid: p.pid, kind: s.kind(), durationSec });
          patchState(s, { captures: [status, ...s.captures().filter((c) => c.id !== status.id)] });
          methods.schedule();
          return status;
        } catch (e) {
          // A refused start may still be listed (failed): show it.
          await methods.poll();
          fail(e);
          return null;
        } finally {
          patchState(s, { busy: false });
        }
      },
      async stop(id: string): Promise<void> {
        await methods.update(id, () => backend.stop(id));
      },
      async cancel(id: string): Promise<void> {
        await methods.update(id, () => backend.cancel(id));
      },
      async update(id: string, call: () => Promise<DotnetCaptureStatus>): Promise<void> {
        patchState(s, { captures: s.captures().map((c) => (c.id === id && c.state === 'recording' ? { ...c, state: 'stopping' as const } : c)) });
        try {
          const status = await call();
          patchState(s, { captures: s.captures().map((c) => (c.id === id ? status : c)) });
          await openFinished([status]);
        } catch (e) {
          fail(e);
        }
      },
    };
    return methods;
  }),
);
