import { Injectable, signal } from '@angular/core';

export const OVERVIEW_AUTO_OPEN_KEY = 'imt.overview.autoOpen.v1';

function readAutoOpen(): boolean {
  try {
    return localStorage.getItem(OVERVIEW_AUTO_OPEN_KEY) !== 'false';
  } catch {
    return true;
  }
}

/** Whether the overview opens by itself after files are opened (a per-browser convenience; default on). */
@Injectable({ providedIn: 'root' })
export class OverviewPrefs {
  private readonly auto = signal(readAutoOpen());
  readonly autoOpen = this.auto.asReadonly();

  setAutoOpen(on: boolean): void {
    this.auto.set(on);
    try {
      localStorage.setItem(OVERVIEW_AUTO_OPEN_KEY, String(on));
    } catch {
      /* storage unavailable: the choice lasts for this session only */
    }
  }
}
