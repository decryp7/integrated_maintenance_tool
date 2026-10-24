import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, inject, Injectable, Injector, resource } from '@angular/core';
import { LifecycleBackend } from '../core/tools-backend';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { DialogService, DIALOG_REF } from '../ui/dialog.service';
import { CHANGELOG } from './changelog.generated';
import { ChangelogRelease, decideWhatsNew, releaseFor } from './changelog';

/** The last version whose notes the user was shown (localStorage: a per-machine convenience). */
export const WHATS_NEW_SEEN_KEY = 'debrief.whatsnew.seen';

/** What's new: the notes of the running version, with earlier releases below. */
@Component({
  selector: 'debrief-whats-new',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <ng-template #notes let-r>
      <h3>
        <span i18n="@@whatsNew.version">Version {{ r.version }}</span>
        @if (r.date) {
          <span class="date">{{ r.date }}</span>
        }
      </h3>
      @for (g of r.groups; track g.title) {
        <h4>{{ g.title }}</h4>
        <ul>
          @for (item of g.items; track item) {
            <li>{{ item }}</li>
          }
        </ul>
      }
    </ng-template>
    <div class="notes">
    @if (version.isLoading()) {
      <!-- The running version comes from the engine; the newest notes are not shown in its place meanwhile. -->
    } @else if (current(); as r) {
      <section class="release" data-testid="whats-new-current" [attr.aria-label]="r.version">
        <ng-container [ngTemplateOutlet]="notes" [ngTemplateOutletContext]="{ $implicit: r }" />
      </section>
    } @else {
      <p class="none" data-testid="whats-new-none" i18n="@@whatsNew.none">There are no release notes for this version.</p>
    }
    @if (earlier().length > 0) {
      <details class="earlier" data-testid="whats-new-earlier">
        <summary i18n="@@whatsNew.earlier">Earlier releases</summary>
        @for (r of earlier(); track r.version) {
          <section class="release">
            <ng-container [ngTemplateOutlet]="notes" [ngTemplateOutletContext]="{ $implicit: r }" />
          </section>
        }
      </details>
    }
    </div>
    <div class="actions">
      <button class="btn primary" type="button" data-testid="whats-new-close" (click)="ref.close()" i18n="@@whatsNew.close">Got it</button>
    </div>
  `,
  imports: [NgTemplateOutlet],
  styles: `
    :host { display: flex; flex-direction: column; gap: var(--s-3); min-width: 0; min-height: 0; max-height: var(--dialog-window-max-height, 70vh); overflow: hidden; }
    /* The notes scroll; the Got it button stays in view whatever the window size (a dialog window has no title-bar buttons). */
    .notes { flex: 1 1 auto; min-height: 0; overflow: auto; }
    h3 { margin: 0 0 var(--s-2); font-size: var(--fs-md); display: flex; gap: var(--s-2); align-items: baseline; }
    h4 { margin: var(--s-3) 0 var(--s-1); font-size: var(--fs-sm); color: var(--text-2); }
    .date { font-size: var(--fs-xs); color: var(--muted); font-weight: 400; }
    ul { margin: 0; padding-inline-start: var(--s-5); display: flex; flex-direction: column; gap: var(--s-1); font-size: var(--fs-sm); line-height: 1.45; }
    .none { margin: 0; color: var(--muted); }
    .earlier summary { cursor: pointer; color: var(--accent-text); }
    .earlier summary:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
    .earlier .release { margin-top: var(--s-3); }
    .actions { display: flex; justify-content: flex-end; }
  `,
})
export class WhatsNewDialog {
  static readonly dialogId = 'whats-new';
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(LifecycleBackend);
  /** The running version, from the engine (`app_status`, as About reads it): a dialog window never starts the lifecycle service. */
  protected readonly version = resource({ loader: () => this.backend.appStatus().then((s) => s.version) });

  /** The running version's notes, or (when it has none, or the engine could not say) the newest release's. */
  protected readonly current = computed<ChangelogRelease | null>(
    () => releaseFor(CHANGELOG, this.version.hasValue() ? this.version.value() : undefined) ?? CHANGELOG[0] ?? null,
  );
  protected readonly earlier = computed(() => CHANGELOG.filter((r) => r !== this.current()));
}

/** Opens What's new on demand (Help menu, palette) and once after an update. */
@Injectable({ providedIn: 'root' })
export class WhatsNewService {
  private readonly dialogs = inject(DialogService);
  // Resolved on use: registering the command must not need the lifecycle backend.
  private readonly injector = inject(Injector);

  open(): void {
    this.dialogs.open(WhatsNewDialog, { title: $localize`:@@whatsNew.title:What's new`, width: 560 });
  }

  /** Shows the notes once per version: after an update, never on a first run. Call when the app status is known. */
  checkOnStart(): void {
    const version = this.injector.get(LifecycleService).status()?.version;
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
