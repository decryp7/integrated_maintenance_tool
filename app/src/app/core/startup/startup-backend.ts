import { inject, Injectable } from '@angular/core';
import { invoke } from '../activity/ipc';
import { Logger, withFailureLogging } from '../logger';
import { browserMocks } from '../mock-registry';
import { isTauri } from '../platform';
import type { ReadyReply } from './startup-contract';
import type { PageMark, StartupReport } from './startup-timings';

/** The start-up contract's port (ADR 0058): ready handshake and the UI state checkpoint a recovery reload restores. */
@Injectable({ providedIn: 'root', useFactory: createStartupBackend })
export abstract class StartupBackend {
  /** Tells Rust the window painted its frame; the reply says whether Debrief reloaded it, and what to restore. */
  abstract reportReady(): Promise<ReadyReply>;
  /** Keeps this window's current UI state in Rust's memory (restored if Debrief has to reload the page). */
  abstract checkpointUi(ui: unknown): Promise<void>;
  /** Tells Rust the main window reached a start-up mark at `atEpochMs` (wall clock). Other windows' marks are ignored. */
  abstract reportMark(mark: PageMark, atEpochMs: number): Promise<void>;
  /** This start's timings so far (Rust's and the page's marks, `setup`'s steps). */
  abstract startupTimings(): Promise<StartupReport>;
}

export class TauriStartupBackend implements StartupBackend {
  reportReady(): Promise<ReadyReply> {
    return invoke<ReadyReply>('window_ready');
  }

  checkpointUi(ui: unknown): Promise<void> {
    return invoke<void>('ui_checkpoint', { ui });
  }

  reportMark(mark: PageMark, atEpochMs: number): Promise<void> {
    return invoke<void>('startup_mark', { mark, atEpochMs });
  }

  startupTimings(): Promise<StartupReport> {
    return invoke<StartupReport>('startup_timings');
  }
}

/** The real backend inside Tauri, the mock in a plain browser; failures are logged. */
export function createStartupBackend(): StartupBackend {
  const real = isTauri() ? new TauriStartupBackend() : new (browserMocks().MockStartupBackend)();
  return withFailureLogging<StartupBackend>(real, inject(Logger));
}
