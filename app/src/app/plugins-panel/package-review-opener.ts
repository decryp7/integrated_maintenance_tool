import { effect, inject, Injectable, untracked } from '@angular/core';
import { DialogRef, DialogService } from '../ui/dialog.service';
import { PluginPackagesStore } from './plugin-packages.store';

/** Opens the review dialog whenever a review starts (once per review, whichever view or drop started it). Inject it to start it. */
@Injectable({ providedIn: 'root' })
export class PackageReviewOpener {
  private readonly dialogs = inject(DialogService);
  private readonly store = inject(PluginPackagesStore);
  private ref: DialogRef | null = null;

  constructor() {
    effect(() => {
      if (this.store.review()) untracked(() => this.open());
    });
  }

  private open(): void {
    if (this.ref) return;
    const ref = this.dialogs.openLazy(() => import('./package-review-dialog').then((m) => m.PackageReviewDialog), { title: $localize`:@@pluginsPanel.packages.review.title:Install plugin`, width: 600 });
    this.ref = ref;
    void ref.closed.then(() => {
      this.ref = null;
      if (this.store.review()) this.store.cancelReview();
    });
  }
}
