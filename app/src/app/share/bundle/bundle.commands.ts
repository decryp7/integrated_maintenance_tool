import { inject, Injectable, Injector } from '@angular/core';
import { DebriefCommand } from '../../extensions/tokens';
import { BundleService } from './bundle.service';

/** Commands for sharing (a saved `.debriefz` project) and old share bundles (services are resolved when a command runs). */
@Injectable({ providedIn: 'root' })
export class BundleCommands {
  private readonly injector = inject(Injector);
  private bundles = () => this.injector.get(BundleService);

  readonly commands: DebriefCommand[] = [
    { id: 'share.bundle.create', title: $localize`:@@share.bundleCommands.shareProject:Share as project file…`, category: $localize`:@@share.commands.categoryFile:File`, when: (ctx) => (ctx.summary?.sources.length ?? 0) > 0, run: () => this.bundles().create() },
    { id: 'share.bundle.open', title: $localize`:@@share.bundleCommands.open:Open bundle…`, category: $localize`:@@share.commands.categoryFile:File`, run: () => this.bundles().openFromDialog() },
  ];
}
