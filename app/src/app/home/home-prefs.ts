import { Injectable, signal } from '@angular/core';

export const HOME_AUTO_OPEN_KEY = 'imt.home.autoOpen.v1';

function readAutoOpen(): boolean {
  try {
    return localStorage.getItem(HOME_AUTO_OPEN_KEY) !== 'false';
  } catch {
    return true;
  }
}

/** Whether Home opens by itself after files or a project are opened (a per-browser convenience; default on: Home is the home screen). */
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
