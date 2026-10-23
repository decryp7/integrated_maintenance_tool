import { DestroyRef, effect, inject, Injectable, Injector } from '@angular/core';
import { Logger } from '../core/logger';
import { ProjectService } from '../projects/project.service';
import { WindowingBackend } from './windowing-backend';

export const DEFAULT_WINDOW_TITLE = 'Debrief';
/** Title changes within this time are sent once (typing a project name, opening several things). */
export const TITLE_DEBOUNCE_MS = 150;

/**
 * The OS window title: "<project> — Debrief" ("● " in front while unsaved, as the project service
 * shows it), or the app's name when no project is open and nothing has changed.
 */
export function windowTitleFor(projectPath: string | null, projectName: string, dirty = false): string {
  if (!projectPath && !dirty) return DEFAULT_WINDOW_TITLE;
  return `${dirty ? '● ' : ''}${projectName} — Debrief`;
}

/** Keeps the OS window title in step with the current project (debounced, sent only when it changes). */
@Injectable({ providedIn: 'root' })
export class WindowTitleSync {
  private readonly backend = inject(WindowingBackend);
  private readonly logger = inject(Logger);
  private readonly injector = inject(Injector);
  private started = false;
  private sent: string | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor() {
    inject(DestroyRef).onDestroy(() => this.cancel());
  }

  start(): void {
    if (this.started) return;
    this.started = true;
    const projects = this.injector.get(ProjectService);
    effect(() => this.schedule(windowTitleFor(projects.path(), projects.name(), projects.dirty())), { injector: this.injector });
  }

  private schedule(title: string): void {
    this.cancel();
    if (title === this.sent) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      this.sent = title;
      this.backend.setWindowTitle(title).catch(() => {
        this.sent = null;
        this.logger.warn('ui.windowing', 'Could not set the window title');
      });
    }, TITLE_DEBOUNCE_MS);
  }

  private cancel(): void {
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }
}
