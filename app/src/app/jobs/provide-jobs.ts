import { effect, EnvironmentProviders, inject, Injectable, makeEnvironmentProviders, provideAppInitializer, signal, untracked } from '@angular/core';
import { Logger } from '../core/logger';
import { IMT_COMMANDS, IMT_VIEWS, ImtCommand, ImtView } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS, UiStateContribution } from '../lifecycle/ui-state';
import { JOBS_VIEW } from './job-viewer';
import { JobsBackend } from './jobs-backend';
import { JobsStore } from './jobs.store';
import { JobsRecord } from './jobs.types';

export function jobsViews(): ImtView[] {
  return [
    {
      id: JOBS_VIEW,
      title: $localize`:View title@@jobs.view.title:Jobs`,
      icon: 'jobs',
      category: 'core',
      defaultPosition: 'bottom',
      area: 'status',
      component: () => import('./jobs-panel').then((m) => m.JobsPanel),
    },
  ];
}

export function jobsCommands(store: InstanceType<typeof JobsStore>): ImtCommand[] {
  const category = $localize`:Command palette category@@jobs.category:View`;
  return [
    { id: 'jobs.show', title: $localize`:@@jobs.cmd.show:View: Jobs`, category, view: JOBS_VIEW, run: (ctx) => ctx.showView(JOBS_VIEW) },
    {
      id: 'jobs.clearFinished',
      title: $localize`:@@jobs.cmd.clear:Jobs: Clear finished`,
      category,
      when: () => store.snapshot().jobs.some((j) => j.state !== 'queued' && j.state !== 'running'),
      run: () => void store.clearFinished(),
    },
  ];
}

/**
 * The window's finished jobs in the project and session (key `jobs`, `JobsRecord` v1). The engine owns the
 * history; this keeps the last copy it returned so saving stays synchronous, and hands saved ones back (the
 * engine validates them; unfinished jobs come back as interrupted).
 */
@Injectable({ providedIn: 'root' })
export class JobsPersistence {
  private readonly backend = inject(JobsBackend);
  private readonly logger = inject(Logger);
  private readonly store = inject(JobsStore);
  readonly record = signal<JobsRecord | null>(null);
  readonly warning = signal<string | null>(null);

  constructor() {
    let finished = '';
    effect(() => {
      const ids = this.store
        .snapshot()
        .jobs.filter((j) => j.state !== 'queued' && j.state !== 'running')
        .map((j) => `${j.id}:${j.unseen}`)
        .join(',');
      if (ids === finished) return;
      finished = ids;
      untracked(() => void this.sync());
    });
  }

  async sync(): Promise<void> {
    try {
      const r = await this.backend.record();
      this.record.set(r.jobs.length ? r : null);
    } catch (e) {
      this.logger.warn('jobs', 'Job history could not be read for saving', { Error: e instanceof Error ? e.message : String(e) });
    }
  }

  async restore(value: unknown): Promise<void> {
    try {
      const r = await this.backend.restore(value ?? null);
      this.warning.set(r.warning);
      this.store.apply(r.snapshot);
      await this.sync();
    } catch (e) {
      this.logger.warn('jobs', 'Saved jobs not restored', { Error: e instanceof Error ? e.message : String(e) });
    }
  }

  readonly contribution: UiStateContribution = {
    key: 'jobs',
    capture: () => this.record(),
    restore: (v) => void this.restore(v),
    reset: () => void this.restore(null),
    tracksChanges: false,
  };
}

/** One line in `app.config.ts`: the Jobs view (View ▸ Jobs), its commands, saved history; running jobs show in the status bar. */
export function provideJobs(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: IMT_VIEWS, multi: true, useFactory: jobsViews },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => jobsCommands(inject(JobsStore)) },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => [inject(JobsPersistence).contribution] },
    provideAppInitializer(() => {
      inject(JobsStore);
    }),
  ]);
}
