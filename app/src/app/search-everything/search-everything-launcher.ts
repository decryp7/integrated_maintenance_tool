import { Dialog, DialogRef } from '@angular/cdk/dialog';
import { Overlay } from '@angular/cdk/overlay';
import { inject, Injectable } from '@angular/core';

/** Opens the search everything palette in a CDK dialog (focus trapped and restored, Escape and the backdrop close it). */
@Injectable({ providedIn: 'root' })
export class SearchEverythingLauncher {
  private readonly dialog = inject(Dialog);
  private readonly overlay = inject(Overlay);
  private ref: Pick<DialogRef<unknown, unknown>, 'close' | 'overlayRef'> | null = null;
  private loading = false;

  /** Whether the palette is open. */
  get isOpen(): boolean {
    return this.ref !== null || this.loading;
  }

  /** Opens the palette near the top of the window; when it is open already, focuses its search box again. */
  open(): void {
    if (this.ref) {
      this.ref.overlayRef.overlayElement.querySelector('input')?.focus();
      return;
    }
    if (this.loading) return;
    // The palette's code is a chunk of its own: it is fetched the first time the palette opens (Paddock rule 35).
    this.loading = true;
    void import('./search-everything-palette').then(
      ({ SearchEverythingPalette }) => {
        this.loading = false;
        if (this.ref) return;
        const ref = this.dialog.open(SearchEverythingPalette, {
          ariaLabel: $localize`:@@search.dialog.label:Search everything`,
          hasBackdrop: true,
          backdropClass: 'cdk-overlay-dark-backdrop',
          panelClass: 'debrief-search-everything-panel',
          positionStrategy: this.overlay.position().global().centerHorizontally().top('72px'),
          autoFocus: 'first-tabbable',
          restoreFocus: true,
        });
        this.ref = ref;
        ref.closed.subscribe(() => (this.ref = null));
      },
      (err: unknown) => {
        this.loading = false;
        console.error('the search palette could not be loaded', err);
      },
    );
  }

  close(): void {
    this.ref?.close();
  }
}
