import { computed, inject, Injectable, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { LifecycleBackend } from '../core/tools-backend';
import { AppStatus, ElevationInfo, Platform } from '../core/tools.types';
import { ProjectService } from '../projects/project.service';
import { errorMessage } from '../tools/tool-errors';
import { SessionService } from './session.service';
import { WindowingBackend } from '../windowing/windowing-backend';

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
  private readonly windowing = inject(WindowingBackend);

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
    let restored = false;
    try {
      const path = await this.backend.lastSession();
      if (!path) {
        this.store.setNotice($localize`:@@lifecycle.service.noSession:There is no saved session to restore`);
      } else if (!(await this.projects.open(path, { session: true }))) {
        return;
      } else {
        restored = true;
      }
      this.dismissCrash();
    } catch (e) {
      this.store.setError($localize`:@@lifecycle.service.restoreFailed:Could not restore the last session: ${errorMessage(e)}:error:`);
    }
    // The previous run's other windows; this window's restore counts even when they cannot reopen.
    if (restored) await this.windowing.restoreWindows().catch((e: unknown) => this.store.setError($localize`:@@lifecycle.service.reopenFailed:Could not reopen the other windows: ${errorMessage(e)}:error:`));
  }

  /** Relaunches without plugins and the saved profile. Nothing is reopened (the old session may be the cause). */
  async startInSafeMode(): Promise<void> {
    await this.run((error) => $localize`:@@lifecycle.service.safeModeFailed:Could not restart in safe mode: ${error}:error:`, () => this.backend.restartApp(true, false));
  }

  async restartNormally(): Promise<void> {
    await this.run((error) => $localize`:@@lifecycle.service.restartFailed:Could not restart: ${error}:error:`, () => this.backend.restartApp(false, false));
  }

  /** Relaunches the app keeping its mode, reopening the session (after a plugin was enabled or disabled). */
  async restartKeepingSession(): Promise<void> {
    await this.run((error) => $localize`:@@lifecycle.service.restartFailed:Could not restart: ${error}:error:`, async () => this.backend.restartApp(this.safeMode(), await this.sessionSaved()));
  }

  /** Saves the session, then relaunches as administrator (Windows UAC prompt) reopening it. */
  async restartElevated(): Promise<void> {
    await this.run((error) => $localize`:@@lifecycle.service.elevatedFailed:Could not restart as administrator: ${error}:error:`, async () => this.backend.restartElevated(await this.sessionSaved()));
  }

  /** Saves the session for the relaunch to reopen; the engine picks the file itself (no path is sent). */
  private async sessionSaved(): Promise<boolean> {
    return this.session.saveNow();
  }

  private async run(failure: (error: string) => string, op: () => Promise<void>): Promise<void> {
    try {
      await op();
    } catch (e) {
      this.store.setError(failure(errorMessage(e)));
    }
  }
}
