import { ChangeDetectionStrategy, Component, computed, inject, input, signal } from '@angular/core';
import { BrowserBackend, ChatBrowserBackend } from './browser-backend';
import { BrowserStore } from './browser.store';
import { AiDriver } from './browser.types';

/**
 * "AI is browsing — Stop" in the Browser panel's header (ADR 0013, amended by ADR 0024): IMT's own UI while a chat's
 * agent mode or a research job drives the page, never anything injected into the page. Who drives comes from Rust
 * (`browser_driving`, then the `driving` events, kept by `BrowserStore`); Stop calls the same stop paths as the chat
 * and the Jobs pane.
 */
@Component({
  selector: 'imt-ai-browsing-banner',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (store.driver(); as d) {
      <div class="banner" role="status" aria-live="polite" data-testid="ai-browsing">
        <span class="dot" aria-hidden="true"></span>
        <strong class="what" i18n="@@browser.banner.title">AI is browsing</strong>
        <span class="detail">{{ progress() || detail() }}</span>
        @if (error(); as e) {
          <span class="err" role="alert" data-testid="ai-browsing-error">{{ e }}</span>
        }
        <button type="button" class="btn primary stop" [disabled]="stopping()" (click)="stop(d)" data-testid="ai-browsing-stop" aria-label="Stop the AI browsing" i18n-aria-label="@@browser.banner.stopLabel" i18n="@@browser.banner.stop">Stop</button>
      </div>
    }
  `,
  styles: `
    :host { display: block; }
    .banner { display: flex; align-items: center; gap: 10px; padding: 4px 8px 4px 10px; background: var(--ann-warning); border-bottom: 1px solid var(--ann-warning-border); color: var(--text); white-space: nowrap; }
    .what { color: var(--warn-text); }
    .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--warn-text); animation: pulse 1.2s ease-in-out infinite; flex: none; }
    .detail, .err { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
    .err { color: var(--danger-text); }
    .stop { margin-left: auto; flex: none; }
    @keyframes pulse { 50% { opacity: 0.3; } }
    @media (prefers-reduced-motion: reduce) { .dot { animation: none; } }
  `,
})
export class AiBrowsingBanner {
  protected readonly store = inject(BrowserStore);
  private readonly browser = inject(BrowserBackend);
  private readonly chat = inject(ChatBrowserBackend);
  /** What the AI does now ("step 3 of 25", "Reading result 2 of 5: learn.microsoft.com"), when the panel knows. */
  readonly progress = input<string | null>(null);
  protected readonly stopping = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly detail = computed(() =>
    this.store.driver()?.kind === 'research'
      ? $localize`:@@browser.banner.research:Research is reading result pages. Leave the page to it, or stop it.`
      : $localize`:@@browser.banner.agent:A chat's agent mode is driving this page. Leave it to the AI, or stop it.`,
  );

  /** Stop: the chat's agent mode ends, or the window's research is cancelled. A failure shows here; Stop retries. */
  async stop(d: AiDriver): Promise<void> {
    this.stopping.set(true);
    this.error.set(null);
    try {
      if (d.kind === 'agent') await this.chat.stop(d.sessionId);
      else await this.browser.researchStop();
    } catch (e) {
      const why = e instanceof Error ? e.message : String(e);
      this.error.set($localize`:@@browser.banner.stopFailed:Could not stop the AI (${why}:reason:). Press Stop to try again.`);
    } finally {
      this.stopping.set(false);
    }
  }
}
