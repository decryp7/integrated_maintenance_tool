import { inject, Injectable, Injector } from '@angular/core';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { DialogService } from '../ui/dialog.service';
import { decideWhatsNew, releaseFor } from './changelog';

/** The last version whose notes the user was shown (localStorage: a per-machine convenience). */
export const WHATS_NEW_SEEN_KEY = 'debrief.whatsnew.seen';

/** Opens What's new on demand (Help menu, palette) and once after an update. */
@Injectable({ providedIn: 'root' })
export class WhatsNewService {
  private readonly dialogs = inject(DialogService);
  // Resolved on use: registering the command must not need the lifecycle backend.
  private readonly injector = inject(Injector);

  open(): void {
    this.dialogs.openLazy(() => import('./whats-new').then((m) => m.WhatsNewDialog), { title: $localize`:@@whatsNew.title:What's new`, width: 560 });
  }

  /** Shows the notes once per version: after an update, never on a first run. Call when the app status is known. */
  async checkOnStart(): Promise<void> {
    const version = this.injector.get(LifecycleService).status()?.version;
    // The notes are a chunk of their own: only a start that may show them needs them.
    const { CHANGELOG } = await import('./changelog.generated');
    const hasNotes = releaseFor(CHANGELOG, version) !== null;
    const decision = decideWhatsNew(this.seen(), version, hasNotes);
    if (decision === 'nothing' || !version) return;
    this.remember(version);
    if (decision === 'show') this.open();
  }

  private seen(): string | null {
    try {
      return localStorage.getItem(WHATS_NEW_SEEN_KEY);
    } catch {
      return null;
    }
  }

  private remember(version: string): void {
    try {
      localStorage.setItem(WHATS_NEW_SEEN_KEY, version);
    } catch {
      /* storage unavailable: the notes may show again next start */
    }
  }
}
