import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { IMT_COMMANDS, ImtCommand } from '../extensions/tokens';
import { DemoService } from './demo.service';
import { TourService } from './tour.service';

/** Help → Try the demo / Take the tour. */
export function onboardingCommands(): ImtCommand[] {
  const demo = inject(DemoService);
  const tour = inject(TourService);
  return [
    { id: 'help.demo', title: 'Try the demo', category: 'Help', run: async () => void (await demo.start()) },
    { id: 'help.tour', title: 'Take the tour', category: 'Help', when: (ctx) => !!ctx.summary?.sources.length, run: () => tour.start() },
  ];
}

/** One line in `app.config.ts`: the demo and the guided tour. */
export function provideOnboarding(): EnvironmentProviders {
  return makeEnvironmentProviders([{ provide: IMT_COMMANDS, multi: true, useFactory: onboardingCommands }]);
}
