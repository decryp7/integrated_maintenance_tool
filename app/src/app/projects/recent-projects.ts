import { InjectionToken } from '@angular/core';

export interface RecentProject {
  path: string;
  name: string;
}

/** Remembers recently used projects, newest first. Implementations must never throw. */
export interface RecentProjects {
  list(): RecentProject[];
  /** Puts a project at the front (deduplicated by path, capped). */
  add(project: RecentProject): void;
  remove(path: string): void;
  clear(): void;
}

export const RECENT_PROJECTS_KEY = 'imt.recentProjects';
export const RECENT_PROJECTS_VERSION = 1;
export const RECENT_PROJECTS_MAX = 10;

/** `localStorage` implementation: a versioned JSON envelope; corrupt data reads as empty. */
export class LocalStorageRecentProjects implements RecentProjects {
  constructor(
    private readonly key = RECENT_PROJECTS_KEY,
    private readonly max = RECENT_PROJECTS_MAX,
  ) {}

  list(): RecentProject[] {
    try {
      const parsed = JSON.parse(localStorage.getItem(this.key) ?? 'null') as { version?: number; projects?: unknown } | null;
      if (!parsed || parsed.version !== RECENT_PROJECTS_VERSION || !Array.isArray(parsed.projects)) return [];
      return parsed.projects
        .filter((p): p is RecentProject => !!p && typeof p.path === 'string' && typeof p.name === 'string')
        .slice(0, this.max);
    } catch {
      return [];
    }
  }

  add(project: RecentProject): void {
    this.write([project, ...this.list().filter((p) => p.path !== project.path)].slice(0, this.max));
  }

  remove(path: string): void {
    this.write(this.list().filter((p) => p.path !== path));
  }

  clear(): void {
    this.write([]);
  }

  private write(projects: RecentProject[]): void {
    try {
      localStorage.setItem(this.key, JSON.stringify({ version: RECENT_PROJECTS_VERSION, projects }));
    } catch {
      /* storage unavailable: recents are a convenience only */
    }
  }
}

export const RECENT_PROJECTS = new InjectionToken<RecentProjects>('RECENT_PROJECTS', {
  providedIn: 'root',
  factory: () => new LocalStorageRecentProjects(),
});
