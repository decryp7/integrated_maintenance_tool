import { Injectable } from '@angular/core';
import { ImtCommand } from '../extensions/tokens';

/** Commands for saved queries (the panel itself is the `saved-queries` view). */
@Injectable({ providedIn: 'root' })
export class SavedQueryCommands {
  readonly commands: ImtCommand[] = [
    {
      id: 'savedQueries.show',
      title: $localize`:@@projects.cmd.savedQueries.show:Show saved queries`,
      category: $localize`:Command palette category|@@projects.cmd.analyzeCategory:Analyze`,
      run: (ctx) => ctx.showView('saved-queries'),
    },
  ];
}
