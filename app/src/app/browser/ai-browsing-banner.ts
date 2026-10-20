import { ChangeDetectionStrategy, Component, computed, EnvironmentProviders, inject, OnDestroy, OnInit, Provider, provideBrowserGlobalErrorListeners, signal } from '@angular/core';
import { provideErrorLogging, provideLogger } from '../core/logger';
import { BrowserBackend, ChatBrowserBackend } from './browser-backend';
import { AiDriver } from './browser.types';

/**
 * "AI is browsing — Stop" on the browser window itself (ADR 0013): IMT's own UI in a small window the shell keeps
 * over the top of the browser window while a chat's agent mode or a research job drives it (never anything
 * injected into the page). Who drives comes from Rust (`browser_driving`, then the `driving` events); Stop calls
 * the same stop paths as the chat and the Web panel.
 */
@Component({
  selector: 'imt-ai-browsing-banner',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (driver(); as d) {
      <div class="banner" role="status" data-testid="ai-browsing">
        <span class="dot" aria-hidden="true"></span>
        <strong class="what" i18n="@@browser.banner.title">AI is browsing</strong>
        <span class="detail">{{ detail() }}</span>
        @if (error(); as e) {
          <span class="err" role="alert" data-testid="ai-browsing-error">{{ e }}</span>
        }
        <button type="button" class="stop" [disabled]="stopping()" (click)="stop(d)" data-testid="ai-browsing-stop" i18n="@@browser.banner.stop">Stop</button>
      </div>
    } @else {
      <div class="banner idle" role="status" data-testid="ai-browsing-idle" i18n="@@browser.banner.idle">The browser is yours again.</div>
    }
  `,
  styles: `
    :host {
      /* A warning colour of its own, the same in dark and light themes: it must stand out from any page. */
      --banner-bg: #f5b800;
      --banner-fg: #1b1b1b;
      --banner-btn-bg: #1b1b1b;
      --banner-btn-fg: #ffffff;
      display: block;
      height: 100vh;
      overflow: hidden;
      background: var(--banner-bg);
      color: var(--banner-fg);
      font: 600 13px/1.2 system-ui, sans-serif;
    }
    .banner { display: flex; align-items: center; gap: 10px; height: 100%; padding: 0 12px; box-sizing: border-box; white-space: nowrap; }
    .idle { font-weight: 400; }
    .dot { width: 10px; height: 10px; border-radius: 50%; background: var(--banner-fg); animation: pulse 1.2s ease-in-out infinite; flex: none; }
    .detail { font-weight: 400; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
    .err { font-weight: 400; overflow: hidden; text-overflow: ellipsis; min-width: 0; text-decoration: underline; }
    .stop { margin-left: auto; flex: none; padding: 4px 14px; border: 0; border-radius: 4px; background: var(--banner-btn-bg); color: var(--banner-btn-fg); font: inherit; cursor: pointer; }
    .stop:focus-visible { outline: 2px solid var(--banner-btn-bg); outline-offset: 2px; }
    .stop:disabled { opacity: 0.7; cursor: default; }
    @keyframes pulse { 50% { opacity: 0.3; } }
    @media (prefers-reduced-motion: reduce) { .dot { animation: none; } }
  `,
})
export class AiBrowsingBanner implements OnInit, OnDestroy {
  private readonly browser = inject(BrowserBackend);
  private readonly chat = inject(ChatBrowserBackend);
  protected readonly driver = signal<AiDriver | null>(null);
  protected readonly stopping = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly detail = computed(() =>
    this.driver()?.kind === 'research'
      ? $localize`:@@browser.banner.research:Research is reading result pages. Leave the browser to it, or stop it.`
      : $localize`:@@browser.banner.agent:A chat's agent mode is driving this browser. Leave it to the AI, or stop it.`,
  );
  private off: (() => void) | undefined;
  private destroyed = false;
  /** An event that came before the first answer of `driving()` wins over it. */
  private heard = false;

  ngOnInit(): void {
    void this.browser
      .onEvent((e) => {
        if (e.type !== 'driving') return;
        this.heard = true;
        this.show(e.by);
      })
      .then((u) => (this.destroyed ? u() : (this.off = u)))
      .catch(() => undefined);
    void this.browser
      .driving()
      .then((d) => {
        if (!this.heard) this.show(d);
      })
      .catch(() => undefined);
  }

  ngOnDestroy(): void {
    this.destroyed = true;
    this.off?.();
  }

  private show(d: AiDriver | null): void {
    this.driver.set(d);
    this.error.set(null);
    this.stopping.set(false);
  }

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

/** The banner window's providers: the backends and logging, nothing of the main window. */
export function aiBrowsingProviders(): (Provider | EnvironmentProviders)[] {
  return [provideBrowserGlobalErrorListeners(), provideLogger(), provideErrorLogging()];
}
