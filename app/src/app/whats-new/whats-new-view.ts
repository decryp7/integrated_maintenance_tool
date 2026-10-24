import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, inject, resource } from '@angular/core';
import { LifecycleBackend } from '../core/tools-backend';
import { CHANGELOG } from './changelog.generated';
import { ChangelogRelease, releaseFor } from './changelog';

/** What's new, a document of the dock: the notes of the running version, with earlier releases below. */
@Component({
  selector: 'debrief-whats-new',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <ng-template #notes let-r>
      <h2>
        <span i18n="@@whatsNew.version">Version {{ r.version }}</span>
        @if (r.date) {
          <span class="date">{{ r.date }}</span>
        }
      </h2>
      @for (g of r.groups; track g.title) {
        <h3>{{ g.title }}</h3>
        <ul>
          @for (item of g.items; track item) {
            <li>{{ item }}</li>
          }
        </ul>
      }
    </ng-template>
    <article class="page" i18n-aria-label="@@whatsNew.title" aria-label="What's new">
      @if (version.isLoading()) {
        <!-- The running version comes from the engine; the newest notes are never shown in its place. -->
      } @else if (current(); as r) {
        <section class="release" data-testid="whats-new-current" [attr.aria-label]="r.version">
          <ng-container [ngTemplateOutlet]="notes" [ngTemplateOutletContext]="{ $implicit: r }" />
        </section>
      } @else {
        <p class="none" data-testid="whats-new-none" i18n="@@whatsNew.none">There are no release notes for this version.</p>
      }
      @if (!version.isLoading() && earlier().length > 0) {
        <details class="earlier" data-testid="whats-new-earlier">
          <summary i18n="@@whatsNew.earlier">Earlier releases</summary>
          @for (r of earlier(); track r.version) {
            <section class="release">
              <ng-container [ngTemplateOutlet]="notes" [ngTemplateOutletContext]="{ $implicit: r }" />
            </section>
          }
        </details>
      }
    </article>
  `,
  imports: [NgTemplateOutlet],
  styles: `
    :host { container: whats-new / inline-size; display: block; height: 100%; min-height: 0; min-width: 0; overflow: auto; background: var(--bg-1); color: var(--text); }
    .page { max-width: 72ch; margin: 0 auto; padding: var(--s-4) var(--s-5) var(--s-6); display: flex; flex-direction: column; gap: var(--s-4); }
    @container whats-new (max-width: 480px) { .page { padding: var(--s-3); } }
    h2 { margin: 0 0 var(--s-2); font-size: var(--fs-lg); display: flex; flex-wrap: wrap; gap: var(--s-2); align-items: baseline; }
    h3 { margin: var(--s-3) 0 var(--s-1); font-size: var(--fs-sm); color: var(--text-2); }
    .date { font-size: var(--fs-xs); color: var(--muted); font-weight: 400; }
    ul { margin: 0; padding-inline-start: var(--s-5); display: flex; flex-direction: column; gap: var(--s-1); font-size: var(--fs-sm); line-height: 1.5; overflow-wrap: anywhere; }
    .none { margin: 0; color: var(--muted); }
    .earlier summary { cursor: pointer; color: var(--accent-text); }
    .earlier summary:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
    .earlier .release { margin-top: var(--s-3); }
  `,
})
export class WhatsNewView {
  private readonly backend = inject(LifecycleBackend);
  /** The running version, from the engine (`app_status`, as About reads it), so a popped-out view knows it too. */
  protected readonly version = resource({ loader: () => this.backend.appStatus().then((s) => s.version) });

  /** The running version's notes; null when it has none or the engine could not say (never the newest release instead). */
  protected readonly current = computed<ChangelogRelease | null>(() => releaseFor(CHANGELOG, this.version.hasValue() ? this.version.value() : undefined));
  protected readonly earlier = computed(() => CHANGELOG.filter((r) => r !== this.current()));
}
