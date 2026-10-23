import type { UpdatesBackend } from './updates-backend';
import { UpdateError, UpdateErrorKind, UpdateInfo, UpdatePhase, UpdateStatus } from './updates.types';

/**
 * Scenarios of the simulated updater, chosen with `?mockUpdate=<name>` (e2e): `available` (default),
 * `upToDate`, `unavailable` (a build without an update key), `offline`, `unreachable`, `signatureInvalid`
 * (the download is refused) and `diskFull` (the download fails for lack of space).
 */
export type MockUpdateScenario = 'available' | 'upToDate' | 'unavailable' | 'offline' | 'unreachable' | 'signatureInvalid' | 'diskFull';

const SCENARIOS: readonly MockUpdateScenario[] = ['available', 'upToDate', 'unavailable', 'offline', 'unreachable', 'signatureInvalid', 'diskFull'];

/** Texts like the engine's (`updater/error.rs`); the mock is not the reference. */
const ERRORS: Record<Exclude<UpdateErrorKind, 'busy' | 'noRelease' | 'noPlatform' | 'invalidManifest' | 'tooLarge' | 'installFailed'>, Omit<UpdateError, 'kind'>> = {
  notAvailable: {
    message: 'Updates are not available in this build.',
    nextStep: 'This build was made without an update signing key, so it cannot verify updates. Download new versions from the releases page.',
    detail: null,
  },
  offline: { message: 'Debrief could not reach the network.', nextStep: 'Check your internet connection or proxy, then try again.', detail: null },
  unreachable: {
    message: 'The update server could not be reached.',
    nextStep: 'Try again later. If it keeps failing, download the new version from the releases page.',
    detail: 'operation timed out',
  },
  signatureInvalid: {
    message: "The update's signature is not valid, so Debrief refused to install it.",
    nextStep: 'The download was discarded. Do not install it by hand; report this to the Debrief maintainers.',
    detail: 'The signature verification failed',
  },
  diskFull: { message: 'There is not enough disk space to install the update.', nextStep: 'Free some disk space, then try again.', detail: null },
};

function error(kind: keyof typeof ERRORS): UpdateError {
  return { kind, ...ERRORS[kind] };
}

export function scenarioOf(search: string): MockUpdateScenario {
  const v = new URLSearchParams(search).get('mockUpdate');
  return SCENARIOS.find((s) => s === v) ?? 'available';
}

const INFO: UpdateInfo = {
  version: '0.2.0',
  currentVersion: '0.1.0',
  notes: '## Features\n- Signed automatic updates\n- <b>Faster</b> grid scrolling\n\n## Fixes\n- Timeline legend overlaps',
  pubDate: '2026-10-17T12:00:00Z',
  size: 48_200_000,
};

/** A simulated updater for browser development and e2e: timers instead of a server. */
export class MockUpdatesBackend implements UpdatesBackend {
  readonly scenario: MockUpdateScenario;
  readonly calls: string[] = [];
  private phase: UpdatePhase;
  private lastChecked: string | null = null;
  private readonly listeners = new Set<(s: UpdateStatus) => void>();
  private timer: ReturnType<typeof setTimeout> | null = null;

  /** `tick` is the simulated latency (ms) of each step. */
  constructor(
    search = '',
    private readonly tick = 120,
  ) {
    this.scenario = scenarioOf(search);
    this.phase = this.scenario === 'unavailable' ? { kind: 'unavailable' } : { kind: 'idle' };
  }

  private snapshot(): UpdateStatus {
    return {
      currentVersion: INFO.currentVersion,
      phase: this.phase,
      lastChecked: this.lastChecked,
      autoCheck: false,
      channel: 'stable',
      endpointHost: this.scenario === 'unavailable' ? null : 'dev.decryptology.net',
      releasesPage: 'https://dev.decryptology.net/decryp7/debrief/releases',
      downloadJob: null,
    };
  }

  private set(phase: UpdatePhase): UpdateStatus {
    this.phase = phase;
    const s = this.snapshot();
    for (const l of this.listeners) l(s);
    return s;
  }

  private refuseUnavailable(): void {
    if (this.phase.kind === 'unavailable') throw error('notAvailable');
  }

  async status(): Promise<UpdateStatus> {
    return this.snapshot();
  }

  async check(): Promise<UpdateStatus> {
    this.calls.push('check');
    this.refuseUnavailable();
    this.set({ kind: 'checking' });
    await new Promise((r) => setTimeout(r, this.tick));
    this.lastChecked = new Date().toISOString();
    switch (this.scenario) {
      case 'upToDate':
        return this.set({ kind: 'upToDate' });
      case 'offline':
      case 'unreachable':
        return this.set({ kind: 'failed', error: error(this.scenario), info: null });
      default:
        return this.set({ kind: 'available', info: INFO });
    }
  }

  async download(): Promise<UpdateStatus> {
    this.calls.push('download');
    this.refuseUnavailable();
    const p = this.phase;
    const info = p.kind === 'available' ? p.info : p.kind === 'failed' ? p.info : null;
    if (!info) throw { kind: 'busy', message: 'There is no update to download.', nextStep: 'Wait for the current step to finish.', detail: null };
    const total = info.size ?? 0;
    const step = (downloaded: number) => {
      if (downloaded >= total / 2 && this.scenario === 'diskFull') return void this.set({ kind: 'failed', error: error('diskFull'), info });
      if (downloaded >= total) {
        this.set(this.scenario === 'signatureInvalid' ? { kind: 'failed', error: error('signatureInvalid'), info } : { kind: 'ready', info, installOnQuit: false });
        return;
      }
      this.set({ kind: 'downloading', info, downloaded, total });
      this.timer = setTimeout(() => step(Math.min(total, downloaded + total / 8)), this.tick);
    };
    step(0);
    return this.snapshot();
  }

  async cancel(): Promise<UpdateStatus> {
    this.calls.push('cancel');
    this.refuseUnavailable();
    const p = this.phase;
    if (p.kind !== 'downloading' && p.kind !== 'ready') throw { kind: 'busy', message: 'There is no download to cancel.', nextStep: '', detail: null };
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    return this.set({ kind: 'available', info: p.info });
  }

  async installOnQuit(enabled: boolean): Promise<UpdateStatus> {
    this.calls.push(`installOnQuit:${enabled}`);
    const p = this.phase;
    if (p.kind !== 'ready') throw { kind: 'busy', message: 'No verified update is ready to install.', nextStep: '', detail: null };
    return this.set({ kind: 'ready', info: p.info, installOnQuit: enabled });
  }

  async restart(): Promise<void> {
    this.calls.push('restart');
    const p = this.phase;
    if (p.kind !== 'ready') throw { kind: 'busy', message: 'No verified update is ready to install.', nextStep: '', detail: null };
    this.set({ kind: 'installing', info: p.info });
  }

  async openReleases(): Promise<void> {
    this.calls.push('openReleases');
  }

  async onStatus(handler: (status: UpdateStatus) => void): Promise<() => void> {
    this.listeners.add(handler);
    return () => this.listeners.delete(handler);
  }
}
