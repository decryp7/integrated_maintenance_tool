import { Injectable, signal } from '@angular/core';
import { isTauri } from '../core/platform';

/** What Window › Full screen (Shift+Alt+Enter) does: hide the tool windows and chrome, make the OS window full screen, or both. */
export type FullScreenMode = 'chrome' | 'os' | 'both';
export const FULL_SCREEN_MODES: readonly FullScreenMode[] = ['both', 'chrome', 'os'];
export const FULL_SCREEN_MODE_KEY = 'debrief.fullScreenMode';

/** The saved full-screen mode ("both" when unset or unreadable). Never throws. */
export function loadFullScreenMode(): FullScreenMode {
  try {
    const v = localStorage.getItem(FULL_SCREEN_MODE_KEY);
    return FULL_SCREEN_MODES.includes(v as FullScreenMode) ? (v as FullScreenMode) : 'both';
  } catch {
    return 'both';
  }
}

/** The window's OS-level state: full screen. Tauri's window API in the app, the Fullscreen API in a browser. */
@Injectable({ providedIn: 'root', useFactory: () => (isTauri() ? new TauriOsWindow() : new BrowserOsWindow()) })
export abstract class OsWindow {
  /** Makes the OS window full screen (or not). Never rejects: a refusal (no permission, no gesture) is ignored. */
  abstract setFullscreen(on: boolean): Promise<void>;
}

export class TauriOsWindow extends OsWindow {
  async setFullscreen(on: boolean): Promise<void> {
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().setFullscreen(on);
    } catch {
      /* the chrome-hidden part still applies */
    }
  }
}

export class BrowserOsWindow extends OsWindow {
  constructor(private readonly doc: Document = document) {
    super();
  }
  async setFullscreen(on: boolean): Promise<void> {
    try {
      if (on && !this.doc.fullscreenElement) await this.doc.documentElement.requestFullscreen?.();
      else if (!on && this.doc.fullscreenElement) await this.doc.exitFullscreen?.();
    } catch {
      /* the browser refused (no user gesture, not allowed): the chrome-hidden part still applies */
    }
  }
}

/** The chosen full-screen mode, remembered in this browser profile like the other UI preferences. */
@Injectable({ providedIn: 'root' })
export class FullScreenPrefs {
  readonly mode = signal<FullScreenMode>(loadFullScreenMode());

  set(mode: FullScreenMode): void {
    this.mode.set(mode);
    try {
      localStorage.setItem(FULL_SCREEN_MODE_KEY, mode);
    } catch {
      /* a convenience only */
    }
  }
}
