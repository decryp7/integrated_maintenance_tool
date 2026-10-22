import { EnvironmentProviders, inject, Injectable, makeEnvironmentProviders, provideEnvironmentInitializer } from '@angular/core';
import { IMT_COMMANDS, ImtCommand } from '../extensions/tokens';
import { UI_STATE_CONTRIBUTIONS, UiStateContribution } from '../lifecycle/ui-state';
import { NavigationHistory } from './navigation.service';

const CATEGORY = $localize`:Command category@@navigation.category:Go`;

/** Back and forward as commands (Go menu items in the palette, Alt+Left / Alt+Right). */
@Injectable({ providedIn: 'root' })
export class NavigationCommands {
  private readonly history = inject(NavigationHistory);

  readonly commands: ImtCommand[] = [
    { id: 'nav.back', title: $localize`:@@navigation.cmd.back:Go back`, category: CATEGORY, keybinding: 'Alt+Left', when: () => this.history.canBack(), run: () => void this.history.back() },
    { id: 'nav.forward', title: $localize`:@@navigation.cmd.forward:Go forward`, category: CATEGORY, keybinding: 'Alt+Right', when: () => this.history.canForward(), run: () => void this.history.forward() },
  ];
}

/** The mouse's back (button 3) and forward (button 4) buttons; returns the listener's remover. */
export function listenMouseNavigation(history: NavigationHistory, target: Window = window): () => void {
  const onUp = (e: MouseEvent) => {
    if (e.button !== 3 && e.button !== 4) return;
    e.preventDefault();
    if (e.button === 3) history.back();
    else history.forward();
  };
  target.addEventListener('mouseup', onUp);
  return () => target.removeEventListener('mouseup', onUp);
}

/** Forgets the places whenever saved UI state is applied: another project's events and ranges are not these. */
export function navigationResetContribution(history: NavigationHistory): UiStateContribution {
  const reset = () => history.clear();
  return { key: 'navigationHistory', tracksChanges: false, capture: () => undefined, restore: reset, reset };
}

/** One line in `app.config.ts`: back/forward commands, recording of places and the mouse buttons. */
export function provideNavigation(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: IMT_COMMANDS, multi: true, useFactory: () => inject(NavigationCommands).commands },
    { provide: UI_STATE_CONTRIBUTIONS, multi: true, useFactory: () => [navigationResetContribution(inject(NavigationHistory))] },
    provideEnvironmentInitializer(() => {
      const history = inject(NavigationHistory);
      history.start();
      listenMouseNavigation(history);
    }),
  ]);
}
