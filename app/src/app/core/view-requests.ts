import { Injectable, signal } from '@angular/core';

/** A request to bring a view to the front. */
export interface ViewRequest {
  id: string;
  nonce: number;
}

/**
 * Domain code (annotations, analyses) asks for a view to come to the front without depending on the dock; the dock's
 * policy (`DockPolicy`) shows it. Each request is new, even for the same view.
 */
@Injectable({ providedIn: 'root' })
export class ViewRequests {
  private nonce = 0;
  private readonly latest = signal<ViewRequest | null>(null);
  readonly request = this.latest.asReadonly();

  show(id: string): void {
    this.latest.set({ id, nonce: ++this.nonce });
  }
}
