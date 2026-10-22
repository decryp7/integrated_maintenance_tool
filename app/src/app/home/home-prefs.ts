import { Injectable, signal } from '@angular/core';

export const HOME_AUTO_OPEN_KEY = 'imt.home.autoOpen.v1';

function readAutoOpen(): boolean {
  try {
    return localStorage.getItem(HOME_AUTO_OPEN_KEY) === 'true';
  } catch {
    return false;
  }
}

/** Whether Home opens by itself after files are opened (a per-browser convenience; default off: the timeline over the events stays the first thing you see). */
@Injectable({ providedIn: 'root' })
export class HomePrefs {
  private readonly auto = signal(readAutoOpen());
  readonly autoOpen = this.auto.asReadonly();

  setAutoOpen(on: boolean): void {
    this.auto.set(on);
    try {
      localStorage.setItem(HOME_AUTO_OPEN_KEY, String(on));
    } catch {
      /* storage unavailable: the choice lasts for this session only */
    }
  }
}
