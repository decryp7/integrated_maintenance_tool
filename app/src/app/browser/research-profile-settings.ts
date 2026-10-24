import { ChangeDetectionStrategy, Component, computed, inject, OnInit } from '@angular/core';
import { ResearchProfileStore } from './research-profile.store';

const mb = (bytes: number) => Math.ceil(bytes / (1 << 20));

/**
 * Settings ▸ Web browser ▸ Research profile: whether research keeps a profile of its own (cookies and consent between
 * researches), its size against its limit, why it was not used last time, and Clear (also the repair of a damaged
 * one). The engine refuses Clear while a research runs or a browser uses the profile; the reason shows inline.
 */
@Component({
  selector: 'debrief-research-profile-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <section class="profile" aria-labelledby="research-profile-title" data-testid="research-profile">
      <h4 id="research-profile-title" i18n="@@browser.profile.title">Research profile</h4>
      @if (store.status(); as s) {
        @if (s.keep) {
          <p data-testid="research-profile-size">{{ sizeText() }}</p>
        } @else {
          <p class="muted" data-testid="research-profile-off" i18n="@@browser.profile.off">Each research starts with a throwaway profile. Turn on Keep a research profile above to remember consent and checks between researches.</p>
        }
        @if (s.problem) {
          <p class="warn" role="note" data-testid="research-profile-problem">{{ s.problem }}</p>
        }
        @if (s.inUse) {
          <p class="muted small" data-testid="research-profile-in-use" i18n="@@browser.profile.inUse">The AI browser uses it now; close its window to clear it.</p>
        }
        @if (s.keep || s.sizeBytes > 0) {
          <div class="row">
            <button type="button" class="btn" [disabled]="store.isLoading() || s.inUse" (click)="store.clear()" data-testid="research-profile-clear" i18n="@@browser.profile.clear">Clear research profile</button>
          </div>
        }
      } @else if (!store.error()) {
        <p class="muted" role="status" i18n="@@browser.profile.loading">Looking at the research profile…</p>
      }
      @if (store.error(); as e) {
        <p class="err" role="alert" data-testid="research-profile-error">{{ e }} <button type="button" class="link" (click)="store.load()" data-testid="research-profile-retry" i18n="@@browser.profile.retry">Retry</button></p>
      }
    </section>
  `,
  styles: `
    :host { display: block; margin-top: 16px; }
    h4 { margin: 0 0 6px; }
    p { margin: 0 0 6px; }
    .row { display: flex; gap: 8px; flex-wrap: wrap; margin-top: 8px; }
    .small { font-size: var(--fs-sm); }
    .muted { color: var(--muted); }
    .warn { color: var(--warn-text); }
    .err { color: var(--danger-text); }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); }
  `,
})
export class ResearchProfileSettings implements OnInit {
  protected readonly store = inject(ResearchProfileStore);

  protected readonly sizeText = computed(() => {
    const s = this.store.status();
    return $localize`:@@browser.profile.size:Research keeps its own profile in Debrief's data folder: ${mb(s?.sizeBytes ?? 0)}:size: MB of ${mb(s?.maxBytes ?? 0)}:max: MB.`;
  });

  ngOnInit(): void {
    void this.store.load();
  }
}
