import { computed, inject, Injectable, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { LifecycleBackend } from '../core/tools-backend';
import { AppStatus, ElevationInfo, Platform } from '../core/tools.types';
import { ProjectService } from '../projects/project.service';
import { errorMessage } from '../tools/tool-errors';
import { SessionService } from './session.service';

/**
 * How this run started and what it can do about it: administrator badge and restart, safe mode,
 * and the offer to restore the previous session after an unclean shutdown.
 */
@Injectable({ providedIn: 'root' })
export class LifecycleService {
  private readonly backend = inject(LifecycleBackend);
  private readonly store = inject(WorkspaceStore);
  private readonly session = inject(SessionService);
  private readonly projects = inject(ProjectService);

  readonly status = signal<AppStatus | null>(null);
  readonly elevation = signal<ElevationInfo | null>(null);
  private readonly crashHandled = signal(false);

  readonly safeMode = computed(() => !!this.status()?.safeMode);
  readonly elevated = computed(() => !!this.elevation()?.elevated);
  /** The operating system the app runs on; null until the backend answered (or when it could not). */
  readonly platform = computed<Platform | null>(() => this.elevation()?.platform ?? null);
  /** Windows-only features (PerfMon collector sets, performance counters, elevation) are available. */
  readonly isWindows = computed(() => this.platform() === 'windows');
  /** "Restart as administrator" is offered (the backend can do it and the app is not already elevated). */
  readonly canRestartElevated = computed(() => !!this.elevation()?.canRestartElevated && !this.elevated());
  /** The previous run ended uncleanly and the user has not decided yet. */
  readonly showCrashBanner = computed(() => !!this.status()?.previousCrash && !this.crashHandled());
  readonly sessionAvailable = computed(() => !!this.status()?.sessionAvailable);

  /** Reads app status and elevation; failures leave the defaults (no banners). */
  async init(): Promise<void> {
    const [status, elevation] = await Promise.allSettled([this.backend.appStatus(), this.backend.elevation()]);
    if (status.status === 'fulfilled') this.status.set(status.value);
    if (elevation.status === 'fulfilled') this.elevation.set(elevation.value);
    // Do not overwrite the session being offered with the empty startup state.
    this.session.enabled.set(!this.showCrashBanner());
  }

  dismissCrash(): void {
    this.crashHandled.set(true);
    this.session.enabled.set(true);
  }

  /** Restores the autosaved session as the workspace and UI state. */
  async restoreLastSession(): Promise<void> {
    try {
      const path = await this.backend.lastSession();
      if (!path) {
        this.store.setNotice('There is no saved session to restore');
      } else if (!(await this.projects.open(path, { session: true }))) {
        return;
      }
      this.dismissCrash();
    } catch (e) {
      this.store.setError(`Could not restore the last session: ${errorMessage(e)}`);
    }
  }

  /** Relaunches without plugins and the saved profile. Nothing is reopened (the old session may be the cause). */
  async startInSafeMode(): Promise<void> {
    await this.run('Could not restart in safe mode', () => this.backend.restartApp(true, []));
  }

  async restartNormally(): Promise<void> {
    await this.run('Could not restart', () => this.backend.restartApp(false, []));
  }

  /** Relaunches the app keeping its mode, reopening the session (after a plugin was enabled or disabled). */
  async restartKeepingSession(): Promise<void> {
    await this.run('Could not restart', async () => this.backend.restartApp(this.safeMode(), await this.sessionPaths()));
  }

  /** Saves the session, then relaunches as administrator (Windows UAC prompt) reopening it. */
  async restartElevated(): Promise<void> {
    await this.run('Could not restart as administrator', async () => this.backend.restartElevated(await this.sessionPaths()));
  }

  private async sessionPaths(): Promise<string[]> {
    if (!(await this.session.saveNow())) return [];
    const path = await this.backend.lastSession();
    return path ? [path] : [];
  }

  private async run(failure: string, op: () => Promise<void>): Promise<void> {
    try {
      await op();
    } catch (e) {
      this.store.setError(`${failure}: ${errorMessage(e)}`);
    }
  }
}
