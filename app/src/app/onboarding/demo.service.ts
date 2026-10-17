import { inject, Injectable, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { UxBackend } from '../core/ux-backend';
import { WelcomeService } from '../welcome/welcome';
import { TourService } from './tour.service';

export const DEMO_MISSING = 'The demo samples are not installed with this copy of IMT. Open your own log files or a folder instead.';

/** "Try a demo": opens the bundled sample incident through the normal open path, then offers the tour. */
@Injectable({ providedIn: 'root' })
export class DemoService {
  private readonly backend = inject(UxBackend);
  private readonly store = inject(WorkspaceStore);
  private readonly welcome = inject(WelcomeService);
  private readonly tour = inject(TourService);
  readonly loading = signal(false);

  /** Resolves true when the demo opened. The tour starts only the first time (Help → Take the tour repeats it). */
  async start(): Promise<boolean> {
    if (this.loading()) return false;
    this.loading.set(true);
    try {
      const paths = await this.backend.demoPaths();
      if (!paths.length) {
        this.store.setError(DEMO_MISSING);
        return false;
      }
      this.welcome.hide();
      await this.store.openPaths(paths);
      if (this.store.error()) return false;
      this.tour.startOnce();
      return true;
    } catch (e) {
      this.store.setError(`Could not open the demo: ${e instanceof Error ? e.message : String(e)}`);
      return false;
    } finally {
      this.loading.set(false);
    }
  }
}
