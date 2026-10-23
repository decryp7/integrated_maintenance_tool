import { computed, inject, Injectable, InjectionToken, Injector, signal } from '@angular/core';
import { isTauri } from '../core/platform';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { DialogService } from '../ui/dialog.service';
import { LanguageStorage, LocalStorageLanguageStorage } from './language-storage';
import { activeLocale, LanguageChoice, resolveLocale } from './locale';

export const LANGUAGE_STORAGE = new InjectionToken<LanguageStorage>('LANGUAGE_STORAGE', {
  providedIn: 'root',
  factory: () => new LocalStorageLanguageStorage(),
});

/**
 * Starts Debrief again so the startup code loads the new language: the desktop app relaunches and reopens the session,
 * the browser build reloads the page. Replaced in tests.
 */
export const PAGE_RELOAD = new InjectionToken<() => void>('PAGE_RELOAD', {
  providedIn: 'root',
  factory: () => {
    const injector = inject(Injector);
    return () => (isTauri() ? void injector.get(LifecycleService).restartKeepingSession() : location.reload());
  },
});

/** The system's preferred languages (replaced in tests). */
export const SYSTEM_LANGUAGES = new InjectionToken<() => readonly string[]>('SYSTEM_LANGUAGES', {
  providedIn: 'root',
  factory: () => () => (navigator.languages?.length ? navigator.languages : [navigator.language]),
});

/**
 * The language preference. Translations are loaded once at startup, so a new choice is saved and applied by
 * reloading the window, which the user confirms first.
 */
@Injectable({ providedIn: 'root' })
export class LanguageService {
  private readonly storage = inject(LANGUAGE_STORAGE);
  private readonly reloadPage = inject(PAGE_RELOAD);
  private readonly systemLanguages = inject(SYSTEM_LANGUAGES);
  private readonly dialogs = inject(DialogService);

  /** The saved choice (what the next start uses). */
  readonly choice = signal<LanguageChoice>(this.storage.load());
  /** The saved choice differs from the language this window runs in: a reload is needed. */
  readonly reloadPending = computed(() => resolveLocale(this.choice(), this.systemLanguages()) !== activeLocale());

  /** Saves the choice and, if it changes the language, offers to reload now. */
  async choose(choice: LanguageChoice): Promise<void> {
    if (choice === this.choice()) return;
    this.choice.set(choice);
    this.storage.save(choice);
    if (!this.reloadPending()) return;
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:Dialog title@@i18n.reload.title:Change the language?`,
      width: 440,
      inputs: {
        message: $localize`:@@i18n.reload.message:Debrief restarts to show the new language and reopens your session. Unsaved edits in open dialogs are lost.`,
        confirmLabel: $localize`:@@i18n.reload.confirm:Restart now`,
        cancelLabel: $localize`:Keep the current language until the next start@@i18n.reload.later:Later`,
      },
    });
    if ((await ref.closed) === true) this.reload();
  }

  reload(): void {
    this.reloadPage();
  }
}
