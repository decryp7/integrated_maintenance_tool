import { RecentKind, RecentLists, RecentProject } from '../api.types';
import { RecentBackend } from './recent-backend';
import { Unlisten } from '../../windowing/window-events';

/** Recent files kept, as Rust keeps them. */
export const MOCK_RECENT_FILES_MAX = 8;
/** Recent projects kept. */
export const MOCK_RECENT_PROJECTS_MAX = 10;
/** Where the browser build keeps its lists (a versioned JSON envelope). */
export const MOCK_RECENT_KEY = 'imt.mock.recent';

/**
 * The browser build's recent lists: what the UI reports it opened, in `localStorage` (survives a reload,
 * like Rust's file). Corrupt or missing data reads as empty; storage errors are ignored.
 */
export class MockRecentBackend extends RecentBackend {
  readonly calls: string[] = [];
  private readonly handlers = new Set<(lists: RecentLists) => void>();

  constructor(private readonly key = MOCK_RECENT_KEY) {
    super();
  }

  async list(): Promise<RecentLists> {
    return this.read();
  }

  async remove(kind: RecentKind, path: string): Promise<void> {
    this.calls.push(`remove:${kind}`);
    const l = this.read();
    if (kind === 'files') l.files = l.files.filter((f) => f !== path);
    else if (kind === 'captures') l.captures = l.captures.filter((f) => f !== path);
    else l.projects = l.projects.filter((p) => p.path !== path);
    this.write(l);
  }

  async clear(kind: RecentKind): Promise<void> {
    this.calls.push(`clear:${kind}`);
    const l = this.read();
    if (kind === 'files') l.files = [];
    else if (kind === 'captures') l.captures = [];
    else l.projects = [];
    this.write(l);
  }

  async opened(files: string[], project?: RecentProject, captures: string[] = []): Promise<void> {
    const l = this.read();
    if (captures.length) l.captures = [...captures, ...l.captures.filter((f) => !captures.includes(f))].slice(0, MOCK_RECENT_FILES_MAX);
    if (files.length) l.files = [...files, ...l.files.filter((f) => !files.includes(f))].slice(0, MOCK_RECENT_FILES_MAX);
    if (project) l.projects = [project, ...l.projects.filter((p) => p.path !== project.path)].slice(0, MOCK_RECENT_PROJECTS_MAX);
    this.write(l);
  }

  async onChanged(handler: (lists: RecentLists) => void): Promise<Unlisten> {
    this.handlers.add(handler);
    return () => this.handlers.delete(handler);
  }

  private read(): RecentLists {
    try {
      const parsed = JSON.parse(localStorage.getItem(this.key) ?? 'null') as { version?: number } & Partial<RecentLists> | null;
      if (!parsed || parsed.version !== 1) return { files: [], projects: [], captures: [] };
      return {
        files: Array.isArray(parsed.files) ? parsed.files.filter((f): f is string => typeof f === 'string') : [],
        projects: Array.isArray(parsed.projects) ? parsed.projects.filter((p): p is RecentProject => !!p && typeof p.path === 'string' && typeof p.name === 'string') : [],
        captures: Array.isArray(parsed.captures) ? parsed.captures.filter((f): f is string => typeof f === 'string') : [],
      };
    } catch {
      return { files: [], projects: [], captures: [] };
    }
  }

  private write(lists: RecentLists): void {
    try {
      localStorage.setItem(this.key, JSON.stringify({ version: 1, ...lists }));
    } catch {
      /* storage unavailable */
    }
    for (const h of this.handlers) h(lists);
  }
}
