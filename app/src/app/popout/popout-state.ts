import { Injectable, signal } from '@angular/core';

/** What a pop-out window shows about its host and itself. */
@Injectable({ providedIn: 'root' })
export class PopoutState {
  /** The host's project name (synced from the host). */
  readonly hostTitle = signal('');
  readonly keepOnTop = signal(false);
}
