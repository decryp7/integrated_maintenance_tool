import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { IMT_COMMANDS, ImtCommand } from '../../extensions/tokens';
import { PROBLEMS_VIEW } from '../../problems/problems-model';
import { KnownIssuesStore } from './known-issues.store';

export function knownIssuesCommands(store: InstanceType<typeof KnownIssuesStore>): ImtCommand[] {
  const category = $localize`:Command palette category@@known.issues.category:View`;
  return [
    {
      id: 'knownIssues.check',
      title: $localize`:@@known.issues.cmd.check:Problems: Check known issues`,
      category,
      view: PROBLEMS_VIEW,
      run: (ctx) => {
        ctx.showView(PROBLEMS_VIEW);
        void store.load();
      },
    },
  ];
}

/** One line in `app.config.ts`: the known-issues feature (ADR 0039): its palette command. The section lives in the Problems pane. */
export function provideKnownIssues(): EnvironmentProviders {
  return makeEnvironmentProviders([{ provide: IMT_COMMANDS, multi: true, useFactory: () => knownIssuesCommands(inject(KnownIssuesStore)) }]);
}
