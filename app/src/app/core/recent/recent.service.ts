import { inject, Injectable, signal } from '@angular/core';
import { RecentKind, RecentProject, RecentLists } from '../api.types';
import { RecentBackend } from './recent-backend';

/**
 * The recent files and projects as signals, kept in step with Rust's lists (loaded once, then
 * updated from the change event). Never throws: the lists are a convenience.
 */
@Injectable({ providedIn: 'root' })
export class RecentService {
  private readonly backend = inject(RecentBackend);
  readonly files = signal<string[]>([]);
  readonly projects = signal<RecentProject[]>([]);
  /** The first load (tests wait for it). */
  readonly ready: Promise<void>;

  constructor() {
    this.ready = this.refresh();
    this.backend.onChanged((lists) => this.apply(lists)).catch(() => undefined);
  }

  async refresh(): Promise<void> {
    try {
      this.apply(await this.backend.list());
    } catch {
      /* the lists stay as they are */
    }
  }

  /** The UI opened files or a project (Rust recorded them; the mock records them now). */
  async opened(files: string[], project?: RecentProject): Promise<void> {
    try {
      await this.backend.opened(files, project);
    } catch {
      /* a convenience only */
    }
    await this.refresh();
  }

  async remove(kind: RecentKind, path: string): Promise<void> {
    try {
      await this.backend.remove(kind, path);
    } catch {
      /* a convenience only */
    }
    await this.refresh();
  }

  async clear(kind: RecentKind): Promise<void> {
    try {
      await this.backend.clear(kind);
    } catch {
      /* a convenience only */
    }
    await this.refresh();
  }

  private apply(lists: RecentLists): void {
    this.files.set(lists.files);
    this.projects.set(lists.projects);
  }
}
