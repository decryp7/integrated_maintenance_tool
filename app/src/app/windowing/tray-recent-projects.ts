import { describeError, Logger, redactPaths } from '../core/logger';
import { RecentProject, RecentProjects } from '../projects/recent-projects';
import { WindowingBackend } from './windowing-backend';

/** The tray menu shows at most this many projects. */
export const TRAY_RECENT_MAX = 10;

/**
 * Decorates a recent-projects store so the tray's "Recent projects" menu follows it: the list is
 * pushed to the shell once when created and after every change. Never throws (the contract of
 * `RecentProjects`); a failed push is logged only.
 */
export class TrayRecentProjects implements RecentProjects {
  constructor(
    private readonly inner: RecentProjects,
    private readonly backend: Pick<WindowingBackend, 'setRecentProjects'>,
    private readonly logger: Logger,
  ) {
    this.push();
  }

  list(): RecentProject[] {
    return this.inner.list();
  }
  add(project: RecentProject): void {
    this.inner.add(project);
    this.push();
  }
  remove(path: string): void {
    this.inner.remove(path);
    this.push();
  }
  clear(): void {
    this.inner.clear();
    this.push();
  }

  private push(): void {
    try {
      const projects = this.inner.list().slice(0, TRAY_RECENT_MAX).map(({ path, name }) => ({ path, name }));
      this.backend.setRecentProjects(projects).catch((e: unknown) => this.failed(e));
    } catch (e) {
      this.failed(e);
    }
  }

  private failed(e: unknown): void {
    this.logger.warn('ui.windowing', 'Could not update the tray recent projects', { Error: redactPaths(describeError(e)) });
  }
}
