import { StartupBackend } from './startup-backend';
import type { Recovery, ReadyReply } from './startup-contract';
import type { PageMark, StartupReport } from './startup-timings';

/** Test hook: set to `terminated` or `unresponsive` before a reload to play "Debrief reloaded this window". */
export const MOCK_RECOVER_KEY = 'debrief.mock.recover';
/** Where the mock keeps the checkpoint across a reload (Rust's memory in the real app). */
export const MOCK_CHECKPOINT_KEY = 'debrief.mock.uiCheckpoint';

function storage(): Storage | null {
  try {
    return typeof sessionStorage === 'undefined' ? null : sessionStorage;
  } catch {
    return null;
  }
}

/** The browser build's start-up port: the same contract as Rust's, with the page's session storage as its memory. */
export class MockStartupBackend implements StartupBackend {
  async reportReady(): Promise<ReadyReply> {
    const s = storage();
    const flag = s?.getItem(MOCK_RECOVER_KEY);
    s?.removeItem(MOCK_RECOVER_KEY);
    const recovered: Recovery | null = flag === 'terminated' || flag === 'unresponsive' ? flag : null;
    let ui: unknown = null;
    if (recovered) {
      try {
        ui = JSON.parse(s?.getItem(MOCK_CHECKPOINT_KEY) ?? 'null');
      } catch {
        ui = null;
      }
    }
    return { recovered, ui };
  }

  async checkpointUi(ui: unknown): Promise<void> {
    storage()?.setItem(MOCK_CHECKPOINT_KEY, JSON.stringify(ui));
  }

  /** The page's marks, in the order they arrived (ms since the page's time origin stands in for the process start). */
  private readonly marks: { mark: PageMark; ms: number }[] = [];

  async reportMark(mark: PageMark, atEpochMs: number): Promise<void> {
    if (this.marks.some((m) => m.mark === mark)) return;
    this.marks.push({ mark, ms: Math.max(0, Math.round(atEpochMs - performance.timeOrigin)) });
  }

  async startupTimings(): Promise<StartupReport> {
    return { marks: [{ mark: 'processStart', ms: 0 }, ...this.marks], steps: [], complete: false };
  }
}
