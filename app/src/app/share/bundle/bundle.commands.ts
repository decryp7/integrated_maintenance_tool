import { inject, Injectable, Injector } from '@angular/core';
import { ImtCommand } from '../../extensions/tokens';
import { BundleService } from './bundle.service';

/** Commands for shareable .imtz bundles (services are resolved when a command runs). */
@Injectable({ providedIn: 'root' })
export class BundleCommands {
  private readonly injector = inject(Injector);
  private bundles = () => this.injector.get(BundleService);

  readonly commands: ImtCommand[] = [
    { id: 'share.bundle.create', title: 'Share as bundle…', category: 'File', when: (ctx) => (ctx.summary?.sources.length ?? 0) > 0, run: () => this.bundles().create() },
    { id: 'share.bundle.open', title: 'Open bundle…', category: 'File', run: () => this.bundles().openFromDialog() },
  ];
}
