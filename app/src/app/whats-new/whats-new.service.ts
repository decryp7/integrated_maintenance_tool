import { effect, inject, Injectable, Injector, untracked } from '@angular/core';
import { DockService } from '../dock/dock.service';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { NotificationsStore } from '../notifications/notifications.store';
import { decideWhatsNew, releaseFor } from './changelog';

/** The last version whose notes the user was shown (localStorage: a per-machine convenience). */
export const WHATS_NEW_SEEN_KEY = 'debrief.whatsnew.seen';

/** The What's new document of the dock. */
export const WHATS_NEW_VIEW = 'whats-new';

/** The cause key of the "Updated to …" notification (ADR 0055): a later update replaces it, opening the view withdraws it. */
export const WHATS_NEW_NOTICE = 'whats-new';

/**
 * Opens What's new on demand (Help menu, palette) and once after an update: the document opens in the background (it
 * never takes the front or the focus) and one calm notification offers to bring it forward.
 */
@Injectable({ providedIn: 'root' })
export class WhatsNewService {
  // Resolved on use: registering the Help command must not need the dock, the lifecycle or the notification backend.
  private readonly injector = inject(Injector);

  /** Opens the view (or brings it to the front); the notification that offered it is done. */
  open(): void {
    this.injector.get(NotificationsStore, null)?.withdrawLocal(WHATS_NEW_NOTICE);
    this.injector.get(DockService).show(WHATS_NEW_VIEW);
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
    if (decision === 'show') this.showWhenDocked(version);
  }

  /** Opens the view in the background once the dock has its layout (restored or default), and posts the notification. */
  private showWhenDocked(version: string): void {
    const dock = this.injector.get(DockService);
    // A side effect on the dock (outside the signal graph), once: the effect destroys itself when it has run.
    const ref = effect(
      () => {
        if (!dock.ready()) return;
        untracked(() => {
          ref.destroy();
          dock.show(WHATS_NEW_VIEW, { inactive: true });
          this.notify(version);
        });
      },
      { injector: this.injector },
    );
  }

  private notify(version: string): void {
    this.injector.get(NotificationsStore, null)?.postLocal({
      key: WHATS_NEW_NOTICE,
      source: 'update',
      severity: 'info',
      title: $localize`:@@whatsNew.notice.title:Updated to ${version}:version:`,
      actionLabel: $localize`:@@whatsNew.notice.action:See what's new`,
      run: () => this.open(),
    });
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
