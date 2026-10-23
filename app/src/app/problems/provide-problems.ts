import { effect, EnvironmentInjector, EnvironmentProviders, inject, makeEnvironmentProviders, provideAppInitializer, untracked } from '@angular/core';
import { DockService } from '../dock/dock.service';
import { DEBRIEF_COMMANDS, DEBRIEF_VIEWS, DebriefCommand, DebriefView } from '../extensions/tokens';
import { PROBLEMS_VIEW, problemsTitle } from './problems-model';
import { ProblemsStore } from './problems.store';

export function problemsViews(): DebriefView[] {
  return [
    {
      id: PROBLEMS_VIEW,
      title: problemsTitle({ critical: 0, warning: 0, info: 0, total: 0 }),
      icon: 'alert',
      category: 'core',
      defaultPosition: 'bottom',
      component: () => import('./problems-panel').then((m) => m.ProblemsPanel),
    },
  ];
}

export function problemsCommands(store: InstanceType<typeof ProblemsStore>): DebriefCommand[] {
  const category = $localize`:Command palette category@@problems.category:View`;
  return [
    { id: 'problems.show', title: $localize`:@@problems.cmd.show:View: Problems`, category, view: PROBLEMS_VIEW, run: (ctx) => ctx.showView(PROBLEMS_VIEW) },
    { id: 'problems.refresh', title: $localize`:@@problems.cmd.refresh:Problems: Refresh`, category, run: () => void store.load() },
  ];
}

/** Keeps the pane's tab title counting the problems ("Problems (3)") while it is open. */
export function startProblemsTitle(): void {
  const injector = inject(EnvironmentInjector);
  const store = inject(ProblemsStore);
  const dock = inject(DockService);
  effect(
    () => {
      const title = problemsTitle(store.counts());
      if (!dock.openIds().has(PROBLEMS_VIEW)) return;
      untracked(() => dock.setTitle(PROBLEMS_VIEW, title));
    },
    { injector },
  );
}

/**
 * One line in `app.config.ts`: the Problems pane (View ▸ Problems, the palette's "View: Problems", the
 * status-bar badge), its counting tab title and the refresh command.
 */
export function provideProblems(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: DEBRIEF_VIEWS, multi: true, useFactory: problemsViews },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => problemsCommands(inject(ProblemsStore)) },
    provideAppInitializer(() => startProblemsTitle()),
  ]);
}
