import { inject, InjectionToken } from '@angular/core';
import { RecentProject } from '../core/api.types';
import { RecentService } from '../core/recent/recent.service';

export type { RecentProject } from '../core/api.types';

/** A project to record: Rust stamps the time. */
export type OpenedProject = Omit<RecentProject, 'openedAt'>;

/** Recently used projects, newest first (Rust's list; the tray shows the same). Implementations must never throw. */
export interface RecentProjects {
  /** Reads a signal where the implementation has one, so `computed` follows changes. */
  list(): RecentProject[];
  /** The UI opened or saved this project (Rust records it itself; see `RecentBackend.opened`). */
  add(project: OpenedProject): void;
  remove(path: string): void;
  clear(): void;
}

/** The recent projects of Rust's list (see `RecentService`). */
export class BackendRecentProjects implements RecentProjects {
  constructor(private readonly recent: RecentService) {}

  list(): RecentProject[] {
    return this.recent.projects();
  }
  add(project: OpenedProject): void {
    void this.recent.opened([], project);
  }
  remove(path: string): void {
    void this.recent.remove('projects', path);
  }
  clear(): void {
    void this.recent.clear('projects');
  }
}

export const RECENT_PROJECTS = new InjectionToken<RecentProjects>('RECENT_PROJECTS', {
  providedIn: 'root',
  factory: () => new BackendRecentProjects(inject(RecentService)),
});
