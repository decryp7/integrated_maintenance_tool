import { ChangeDetectionStrategy, Component, computed, inject, OnInit } from '@angular/core';
import { BrowserRuntimeStore } from './browser-runtime.store';

/**
 * Settings ▸ Web browser ▸ Browser for research: which Chromium-based browser research and the chat's browser modes
 * drive, and the optional download of a known-good Chromium (only on the user's request; the engine verifies its
 * checksum). The engine decides which browser is used; this shows it.
 */
@Component({
  selector: 'imt-browser-runtime-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <section class="runtime" aria-labelledby="browser-runtime-title" data-testid="browser-runtime">
      <h4 id="browser-runtime-title" i18n="@@browser.runtime.title">Browser for research</h4>
      @if (store.status(); as s) {
        @if (s.available) {
          <p data-testid="browser-runtime-status">{{ sourceText() }}</p>
          @if (s.path) {
            <p class="mono small muted path" data-testid="browser-runtime-path">{{ s.path }}</p>
          }
        } @else {
          <p class="warn" role="note" data-testid="browser-runtime-status">{{ missingText() }}</p>
        }
        @if (outdated()) {
          <p class="muted small" data-testid="browser-runtime-outdated">{{ outdatedText() }}</p>
        }
        <div class="row">
          @if (s.canInstall && !s.installedVersion) {
            <button type="button" class="btn primary" [disabled]="!!store.busy()" (click)="store.install()" data-testid="browser-runtime-install">{{ store.busy() === 'install' ? downloading : download }}</button>
          }
          @if (outdated()) {
            <button type="button" class="btn primary" [disabled]="!!store.busy()" (click)="store.install()" data-testid="browser-runtime-update">{{ store.busy() === 'install' ? downloading : update }}</button>
          }
          @if (s.installedVersion) {
            <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.remove()" data-testid="browser-runtime-remove" i18n="@@browser.runtime.remove">Remove downloaded Chromium</button>
          }
        </div>
        @if (store.busy() === 'install') {
          <p class="muted small" role="status" data-testid="browser-runtime-busy" i18n="@@browser.runtime.busy">Downloading Chromium (about 200 MB). You can keep working; this can take a few minutes.</p>
        }
      } @else if (!store.error()) {
        <p class="muted" role="status" i18n="@@browser.runtime.loading">Looking for a browser…</p>
      }
      @if (store.error(); as e) {
        <p class="err" role="alert" data-testid="browser-runtime-error">{{ e }} <button type="button" class="link" (click)="retry()" data-testid="browser-runtime-retry" i18n="@@browser.runtime.retry">Retry</button></p>
      }
    </section>
  `,
  styles: `
    :host { display: block; margin-top: 16px; }
    h4 { margin: 0 0 6px; }
    p { margin: 0 0 6px; }
    .row { display: flex; gap: 8px; flex-wrap: wrap; margin-top: 8px; }
    .small { font-size: var(--fs-sm); }
    .path { overflow-wrap: anywhere; }
    .warn { color: var(--warn-text); }
    .err { color: var(--danger-text); }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); }
  `,
})
export class BrowserRuntimeSettings implements OnInit {
  protected readonly store = inject(BrowserRuntimeStore);
  protected readonly download = $localize`:@@browser.runtime.download:Download Chromium`;
  protected readonly downloading = $localize`:@@browser.runtime.downloading:Downloading…`;
  protected readonly update = $localize`:@@browser.runtime.update:Update Chromium`;

  /** A downloaded Chromium that is not the pinned version. */
  protected readonly outdated = computed(() => {
    const s = this.store.status();
    return !!s?.installedVersion && !!s.canInstall && s.installedVersion !== s.pinnedVersion;
  });

  protected readonly sourceText = computed(() => {
    switch (this.store.status()?.source) {
      case 'configured':
        return $localize`:@@browser.runtime.configured:Research uses the browser you chose in Browser to use.`;
      case 'installed':
        return $localize`:@@browser.runtime.installed:Research uses the Chromium IMT downloaded.`;
      default:
        return $localize`:@@browser.runtime.found:Research uses the browser found on this computer.`;
    }
  });

  protected readonly missingText = computed(() => {
    const v = this.store.status()?.pinnedVersion ?? '';
    return this.store.status()?.canInstall
      ? $localize`:@@browser.runtime.missing:No Chromium-based browser found: research and the browser modes are unavailable. Install Edge, Chrome or Chromium, or download a known-good Chromium (about 200 MB, pinned version ${v}:version:, checksum verified).`
      : $localize`:@@browser.runtime.missingNoDownload:No Chromium-based browser found: research and the browser modes are unavailable. Install Edge, Chrome or Chromium, or choose one in Browser to use.`;
  });

  protected readonly outdatedText = computed(() => {
    const s = this.store.status();
    return $localize`:@@browser.runtime.outdated:The downloaded Chromium is ${s?.installedVersion ?? ''}:installed:; the known-good version is ${s?.pinnedVersion ?? ''}:pinned:.`;
  });

  ngOnInit(): void {
    void this.store.load();
  }

  protected retry(): void {
    void this.store.load();
  }
}
