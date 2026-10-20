import { DOCUMENT, effect, inject, Injectable, Injector } from '@angular/core';
import { UiPrefsStore } from './ui-prefs.store';

/** Applies the theme preference to the document (`data-theme` on <html>; none follows the system). */
@Injectable({ providedIn: 'root' })
export class ThemeService {
  private readonly prefs = inject(UiPrefsStore);
  private readonly root = inject(DOCUMENT).documentElement;
  private readonly injector = inject(Injector);
  private started = false;

  /** Starts following the preference; idempotent. */
  start(): void {
    if (this.started) return;
    this.started = true;
    effect(
      () => {
        const theme = this.prefs.theme();
        if (theme === 'system') this.root.removeAttribute('data-theme');
        else this.root.setAttribute('data-theme', theme);
      },
      { injector: this.injector },
    );
  }
}
