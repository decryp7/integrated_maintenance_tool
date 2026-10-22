import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { Icon } from '../shared/icon';
import { BrowserBackend, ChatBrowserBackend } from './browser-backend';
import { BrowserActivityStore } from './browser-activity.store';
import { AiDriver } from './browser.types';

/**
 * "The AI is browsing — Stop" in the status bar while a chat's agent mode or a research job drives the browser window
 * (ADR 0013, amended): Debrief's own UI, never anything injected into a page. What it shows comes from Rust through
 * `BrowserActivityStore`. Stop ends the chat's agent mode or the window's research (the same paths as the chat and the
 * Jobs pane) and closes the AI's browser window at once.
 */
@Component({
  selector: 'debrief-ai-browsing-chip',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    @if (store.driver(); as d) {
      <span class="chip" [class.waiting]="waiting()" role="status" aria-live="polite" data-testid="ai-browsing" [attr.data-waiting]="waiting()">
        <span class="dot" aria-hidden="true"></span>
        <span class="what" data-testid="ai-browsing-text">{{ line() }}</span>
        @if (host(); as h) {
          @if (!waiting()) {
            <span class="host" data-testid="ai-browsing-host">{{ h }}</span>
          }
        }
        @if (error(); as e) {
          <span class="err" role="alert" data-testid="ai-browsing-error">{{ e }}</span>
        }
        <button type="button" class="stop" [disabled]="stopping()" (click)="stop(d)" data-testid="ai-browsing-stop" aria-label="Stop the AI's browser" i18n-aria-label="@@browser.chip.stopLabel" title="Stop the AI's browser" i18n-title="@@browser.chip.stopTitle">
          <debrief-icon name="close" [size]="11" />
        </button>
      </span>
    }
  `,
  styles: `
    :host { display: contents; }
    .chip { display: inline-flex; align-items: center; gap: 6px; height: 18px; padding: 0 3px 0 8px; border-radius: 9px; border: 1px solid var(--ann-warning-border); background: var(--ann-warning); color: var(--text); font-size: var(--fs-xs); min-width: 0; max-width: 360px; flex: none; }
    .chip.waiting { border-style: dashed; border-color: var(--accent); background: var(--accent-soft); font-weight: 600; }
    .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--warn-text); animation: pulse 1.2s ease-in-out infinite; flex: none; }
    .waiting .dot { background: var(--accent); animation: none; }
    .what, .host, .err { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
    .host { color: var(--muted); }
    .err { color: var(--danger-text); }
    .stop { all: unset; box-sizing: border-box; display: inline-flex; align-items: center; justify-content: center; width: 14px; height: 14px; border-radius: 7px; cursor: pointer; flex: none; }
    .stop:hover:not(:disabled) { background: var(--bg-hover); }
    .stop:focus-visible { outline: 2px solid var(--focus-ring); }
    .stop:disabled { opacity: 0.5; cursor: default; }
    @keyframes pulse { 50% { opacity: 0.3; } }
    @media (prefers-reduced-motion: reduce) { .dot { animation: none; } }
  `,
})
export class AiBrowsingChip {
  protected readonly store = inject(BrowserActivityStore);
  private readonly browser = inject(BrowserBackend);
  private readonly chat = inject(ChatBrowserBackend);
  protected readonly stopping = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly waiting = computed(() => this.store.activity() === 'waitingForYou');
  protected readonly host = computed(() => this.store.host());
  /** The engine's progress line; else what the window does ("Reading"); else that the AI browses. */
  protected readonly line = computed(() => {
    const host = this.store.host() ?? '';
    switch (this.store.activity()) {
      case 'waitingForYou':
        return $localize`:@@browser.chip.waiting:Waiting for you in the browser window: ${host}:host:`;
      case 'reading':
        return this.store.text() || $localize`:@@browser.chip.reading:Reading`;
      case 'searching':
        return this.store.text() || $localize`:@@browser.chip.searching:Searching`;
      default:
        return this.store.text() || $localize`:@@browser.chip.browsing:The AI is browsing`;
    }
  });

  /** Stop: the chat's agent mode ends or the window's research is cancelled, then the window closes. A failure shows here; Stop retries. */
  async stop(d: AiDriver): Promise<void> {
    this.stopping.set(true);
    this.error.set(null);
    try {
      try {
        if (d.kind === 'agent') await this.chat.stop(d.sessionId);
        else await this.browser.researchStop();
      } finally {
        await this.browser.stopAll();
      }
    } catch (e) {
      const why = e instanceof Error ? e.message : String(e);
      this.error.set($localize`:@@browser.chip.stopFailed:Could not stop the AI (${why}:reason:). Press Stop to try again.`);
    } finally {
      this.stopping.set(false);
    }
  }
}
