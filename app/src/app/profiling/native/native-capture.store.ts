import { computed, inject, Injectable, Injector } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { NativeCaptureMode, NativeCaptureRequest, NativeCaptureStatus, NativeProcess } from '../../core/api.types';
import { CmdError } from '../../core/cmd-error';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { errorMessage } from '../../tools/tool-errors';
import { CapturesStore } from '../captures.store';
import { PROFILING_VIEW_IDS as V } from '../profiling-navigator';
import { showOpened } from '../provide-profiling';
import { NativeBackend } from './native-backend';

/** How often running captures are polled. */
export const POLL_MS = 1000;
/** Sampling rate bounds (per thread; the plugin checks them again). */
export const MIN_RATE_HZ = 1;
export const MAX_RATE_HZ = 1000;

/** Opens a finished capture problem-first: its findings. Replaceable in tests. */
@Injectable({ providedIn: 'root' })
export class NativeOpener {
  private readonly captures = inject(CapturesStore);
  private readonly injector = inject(Injector);

  async open(path: string): Promise<void> {
    const opened = await this.captures.openPaths([path]);
    const ctx = this.injector.get(COMMAND_CONTEXT);
    showOpened(ctx, opened);
    if (opened.length) ctx.showView(V.findings);
  }
}

/** Whether a capture still runs. */
export function isRunning(c: NativeCaptureStatus): boolean {
  return c.state === 'recording' || c.state === 'stopping';
}

/** Case-insensitive match on name, pid or path. */
export function matchesProcess(p: NativeProcess, filter: string): boolean {
  const f = filter.trim().toLowerCase();
  return !f || p.name.toLowerCase().includes(f) || String(p.pid).includes(f) || (p.path ?? '').toLowerCase().includes(f);
}

/** A whole number in `min..=max`, or `fallback` for anything else. */
export function clampInt(v: number, min: number, max: number, fallback: number): number {
  return Number.isFinite(v) ? Math.min(max, Math.max(min, Math.round(v))) : fallback;
}

interface NativeState {
  processes: NativeProcess[];
  loading: boolean;
  error: string | null;
  /** The error's `CmdError` code (`denied`: offer the restart as administrator or the other mode). */
  errorCode: string | null;
  filter: string;
  selectedPid: number | null;
  mode: NativeCaptureMode;
  /** null: until stopped. */
  durationSec: number | null;
  rateHz: number;
  /** ETW only: heap allocations with their stacks. */
  heap: boolean;
  symbolPath: string;
  symbolServer: boolean;
  captures: NativeCaptureStatus[];
  /** Captures already opened (each opens once). */
  opened: string[];
  /** An ETW capture was started: its (elevated) tool is polled too. Before that, never (no administrator prompt). */
  etwUsed: boolean;
  busy: boolean;
}

const initial: NativeState = {
  processes: [],
  loading: false,
  error: null,
  errorCode: null,
  filter: '',
  selectedPid: null,
  mode: 'sampling',
  durationSec: 30,
  rateHz: 100,
  heap: false,
  symbolPath: '',
  symbolServer: false,
  captures: [],
  opened: [],
  etwUsed: false,
  busy: false,
};

/** "Profile native process…": the process list, the capture choice and the running/finished captures. */
export const NativeCaptureStore = signalStore(
  { providedIn: 'root' },
  withState(initial),
  withComputed((s) => ({
    visible: computed(() => s.processes().filter((p) => matchesProcess(p, s.filter()))),
    selected: computed(() => s.processes().find((p) => p.pid === s.selectedPid()) ?? null),
    running: computed(() => s.captures().filter(isRunning)),
    modes: computed<NativeCaptureMode[]>(() => (s.etwUsed() ? ['sampling', 'etw'] : ['sampling'])),
  })),
  withComputed((s) => ({
    /** Sampling cannot open this process (elevated or another user's): only ETW can record it. */
    needsEtw: computed(() => s.mode() === 'sampling' && s.selected()?.accessible === false),
  })),
  withComputed((s) => ({
    canRecord: computed(() => !!s.selected() && !s.busy() && !s.needsEtw() && !s.running().some((c) => c.pid === s.selectedPid())),
  })),
  withMethods((s, backend = inject(NativeBackend), opener = inject(NativeOpener)) => {
    const fail = (e: unknown) => patchState(s, { error: errorMessage(e), errorCode: e instanceof CmdError ? e.code : null });
    let timer: ReturnType<typeof setTimeout> | null = null;
    let defaultsLoaded = false;

    const openFinished = async (list: NativeCaptureStatus[]) => {
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
          if (!defaultsLoaded) {
            const d = await backend.defaults();
            defaultsLoaded = true;
            patchState(s, { symbolPath: d.symbolPath, symbolServer: d.symbolServer, rateHz: d.rateHz });
          }
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
      setMode(mode: NativeCaptureMode): void {
        patchState(s, { mode });
      },
      setDuration(durationSec: number | null): void {
        patchState(s, { durationSec: durationSec === null ? null : clampInt(durationSec, 1, 3600, 30) });
      },
      setRate(rateHz: number): void {
        patchState(s, { rateHz: clampInt(rateHz, MIN_RATE_HZ, MAX_RATE_HZ, 100) });
      },
      setHeap(heap: boolean): void {
        patchState(s, { heap });
      },
      setSymbolPath(symbolPath: string): void {
        patchState(s, { symbolPath: symbolPath.slice(0, 4096) });
      },
      setSymbolServer(symbolServer: boolean): void {
        patchState(s, { symbolServer });
      },
      dismissError(): void {
        patchState(s, { error: null, errorCode: null });
      },
      /** The request for the current choice (each mode sends only its own options). */
      request(pid: number): NativeCaptureRequest {
        const base: NativeCaptureRequest = { pid, durationSec: s.durationSec(), symbolPath: s.symbolPath().trim(), symbolServer: s.symbolServer() };
        return s.mode() === 'etw' ? { ...base, heap: s.heap() } : { ...base, rateHz: s.rateHz() };
      },
      /** Reads the captures; opens those that just finished; keeps polling while any runs. */
      async poll(): Promise<void> {
        try {
          const captures = await backend.captures(s.modes());
          // Captures that existed before this dialog opened are not opened again.
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
      async start(): Promise<NativeCaptureStatus | null> {
        const p = s.selected();
        if (!p || !s.canRecord()) return null;
        const mode = s.mode();
        patchState(s, { busy: true, error: null, errorCode: null });
        try {
          const status = await backend.start(mode, methods.request(p.pid));
          if (mode === 'etw') patchState(s, { etwUsed: true });
          patchState(s, { captures: [status, ...s.captures().filter((c) => c.id !== status.id)] });
          methods.schedule();
          return status;
        } catch (e) {
          fail(e);
          return null;
        } finally {
          patchState(s, { busy: false });
        }
      },
      async stop(c: NativeCaptureStatus): Promise<void> {
        await methods.update(c.id, () => backend.stop(c.mode, c.id));
      },
      async cancel(c: NativeCaptureStatus): Promise<void> {
        await methods.update(c.id, () => backend.cancel(c.mode, c.id));
      },
      async update(id: string, call: () => Promise<NativeCaptureStatus>): Promise<void> {
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
