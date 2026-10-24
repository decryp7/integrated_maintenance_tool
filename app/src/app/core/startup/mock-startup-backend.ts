import { StartupBackend } from './startup-backend';
import type { Recovery, ReadyReply } from './startup-contract';

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
}
