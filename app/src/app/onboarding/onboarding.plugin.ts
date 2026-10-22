import { EnvironmentProviders, inject, makeEnvironmentProviders } from '@angular/core';
import { DEBRIEF_COMMANDS, DebriefCommand } from '../extensions/tokens';
import { DemoService } from './demo.service';
import { TourService } from './tour.service';

/** Help → Try the demo / Take the tour. */
export function onboardingCommands(): DebriefCommand[] {
  const demo = inject(DemoService);
  const tour = inject(TourService);
  return [
    { id: 'help.demo', title: $localize`:@@onboarding.command.demo:Try the demo`, category: $localize`:@@onboarding.command.category:Help`, run: async () => void (await demo.start()) },
    { id: 'help.tour', title: $localize`:@@onboarding.command.tour:Take the tour`, category: $localize`:@@onboarding.command.category:Help`, when: (ctx) => !!ctx.summary?.sources.length, run: () => tour.start() },
  ];
}

/** One line in `app.config.ts`: the demo and the guided tour. */
export function provideOnboarding(): EnvironmentProviders {
  return makeEnvironmentProviders([{ provide: DEBRIEF_COMMANDS, multi: true, useFactory: onboardingCommands }]);
}
