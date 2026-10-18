import { inject, Injectable } from '@angular/core';
import { Logger } from '../core/logger';
import { WorkspaceStore } from '../core/workspace.store';
import { ProjectBackend, Unlisten } from '../core/tools-backend';
import { baseName, errorMessage } from '../tools/tool-errors';
import { isProjectPath, ProjectService } from './project.service';
import { BundleService, isBundlePath } from '../share/bundle/bundle.service';

const LOAD_WAIT_TRIES = 100;
const LOAD_WAIT_MS = 20;

/** File name of the autosaved session (see lifecycle.rs); opened as a session, never as the current project. */
export const SESSION_FILE_NAME = 'last-session.imtproj';

/**
 * Files the OS asks IMT to open (file association, "open with", command line). `.imtproj` files
 * open as projects, `.imtz` bundles open read-only; everything else is added to the workspace. (Live-tail refreshes are handled
 * by `LiveTail` only.)
 */
@Injectable({ providedIn: 'root' })
export class OpenRequests {
  private readonly backend = inject(ProjectBackend);
  private readonly projects = inject(ProjectService);
  private readonly store = inject(WorkspaceStore);
  private readonly logger = inject(Logger);
  private readonly bundles = inject(BundleService);
  private unlisten: Unlisten | null = null;

  /** Handles what is already waiting and subscribes to new requests. Idempotent. */
  async start(): Promise<void> {
    if (this.unlisten) return;
    await this.whenWorkspaceLoaded();
    try {
      this.unlisten = await this.backend.onOpenRequested(() => void this.drain());
    } catch (e) {
      this.logger.warn('ui.open', 'Could not subscribe to open requests', { Error: errorMessage(e) });
      return;
    }
    await this.drain();
  }

  stop(): void {
    this.unlisten?.();
    this.unlisten = null;
  }

  /** Takes the queued paths from the shell and opens them. */
  async drain(): Promise<void> {
    try {
      await this.handle(await this.backend.takePendingOpen());
    } catch (e) {
      this.store.setError($localize`:@@projects.openRequests.failed:Could not open the requested files: ${errorMessage(e)}:error:`);
    }
  }

  /** Opens the first project (or else the first `.imtz` bundle, read-only), then adds the remaining files to the workspace. */
  async handle(paths: string[]): Promise<void> {
    const project = paths.find(isProjectPath);
    const bundle = paths.find(isBundlePath);
    if (project) await this.projects.open(project, { session: baseName(project).toLowerCase() === SESSION_FILE_NAME });
    else if (bundle) await this.bundles.open(bundle);
    const others = paths.filter((p) => !isProjectPath(p) && !isBundlePath(p));
    if (others.length) await this.store.openPaths(others);
  }

  /** The first workspace load must not overwrite files opened at startup, so wait for it (bounded). */
  private async whenWorkspaceLoaded(): Promise<void> {
    for (let i = 0; i < LOAD_WAIT_TRIES && !this.store.summary(); i++) await new Promise((r) => setTimeout(r, LOAD_WAIT_MS));
  }
}
