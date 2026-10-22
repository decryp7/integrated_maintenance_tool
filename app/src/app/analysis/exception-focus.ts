import { Injectable, signal } from '@angular/core';

/**
 * A request, from elsewhere in the app (the search palette), to select one exception group in the Exceptions
 * view. The view applies it once its groups are loaded and then clears it, so it never re-applies on a later visit.
 */
@Injectable({ providedIn: 'root' })
export class ExceptionFocus {
  private readonly pending = signal<string | null>(null);
  /** The fingerprint waiting to be selected, if any. */
  readonly fingerprint = this.pending.asReadonly();

  request(fingerprint: string): void {
    this.pending.set(fingerprint);
  }

  clear(): void {
    this.pending.set(null);
  }
}
