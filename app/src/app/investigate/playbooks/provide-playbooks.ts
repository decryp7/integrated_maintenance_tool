import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { DEBRIEF_COMMANDS, DEBRIEF_VIEWS, DebriefCommand, DebriefView } from '../../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS } from '../../lifecycle/ui-state';
import { PLAYBOOKS_VIEW } from './playbook-model';
import { PlaybookPersistence } from './playbook-persistence';
import { PlaybookStore } from './playbook.store';

export function playbooksViews(): DebriefView[] {
  return [
    {
      id: PLAYBOOKS_VIEW,
      title: $localize`:@@playbooks.view.title:Playbooks`,
      icon: 'check',
      category: 'core',
      defaultPosition: 'right', layout: { area: 'side', min: { width: 320, height: 240 }, pref: { width: 372, height: 400 }, priority: 45 },
      component: () => import('./playbooks-panel').then((m) => m.PlaybooksPanel),
    },
  ];
}

export function playbooksCommands(store: InstanceType<typeof PlaybookStore>): DebriefCommand[] {
  const category = $localize`:Command palette category@@playbooks.category:View`;
  return [
    { id: 'playbooks.show', title: $localize`:@@playbooks.cmd.show:View: Playbooks`, category, view: PLAYBOOKS_VIEW, run: (ctx) => ctx.showView(PLAYBOOKS_VIEW) },
    {
      id: 'playbooks.new',
      title: $localize`:@@playbooks.cmd.new:Playbooks: New playbook`,
      category,
      run: (ctx) => {
        ctx.showView(PLAYBOOKS_VIEW);
        store.startNew();
      },
    },
  ];
}

/** One line in `app.config.ts`: the Playbooks pane, its commands and the runs saved in the project (ADR 0039). */
export function providePlaybooks(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: DEBRIEF_VIEWS, multi: true, useFactory: playbooksViews },
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => playbooksCommands(inject(PlaybookStore)) },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => [inject(PlaybookPersistence).contribution] },
  ]);
}
