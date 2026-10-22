import { ChangeDetectionStrategy, Component, computed, effect, inject, input, untracked } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { ChatBrowserStore } from './chat-browser.store';
import { BrowserLauncher } from './provide-browser';

/**
 * The browser in the chat panel, above the composer: pages added for the next question (untrusted web content), what
 * agent mode may do while it is on (the composer's "Browser" menu turns it on and off; Stop ends it), and "Search the
 * web for the selected event".
 */
@Component({
  selector: 'imt-chat-browser-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="bar" data-testid="chat-browser-bar">
      @if (sources().length) {
        <ul class="sources" aria-label="Web pages for your next question" i18n-aria-label="@@browser.chatBar.sources.label">
          @for (s of sources(); track s.url) {
            <li class="chip src" [title]="s.url" data-testid="chat-source"><span class="ellipsis">{{ s.title }}</span><span class="tag" i18n="@@browser.chatBar.untrusted">untrusted web content</span></li>
          }
          <li><button type="button" class="btn ghost" (click)="store.discardSources(sessionId())" i18n="@@browser.chatBar.discard">Remove pages</button></li>
        </ul>
      }
      @if (selectedId() !== null) {
        <div class="row">
          <button type="button" class="btn ghost" (click)="searchSelected()" data-testid="chat-search-web" i18n="@@browser.chatBar.search">Search the web for the selected event…</button>
        </div>
      }
      @if (status()?.agent) {
        <p class="note small" i18n="@@browser.chatBar.agentNote">The AI may drive IMT's browser one step at a time. New sites, typing and forms need your approval; it stops for logins, payments and CAPTCHAs.</p>
        @if (status()?.smallModel) {
          <p class="hint small" role="note" data-testid="chat-small-model" i18n="@@browser.chatBar.smallModel">Agent mode works best with a 7B or larger model; yours may lose track on long tasks.</p>
        }
      }
      @if (store.error(); as e) {
        <p class="err small" role="alert">{{ e }} <button type="button" class="link" (click)="retry()" i18n="@@browser.chatBar.retry">Retry</button></p>
      }
    </div>
  `,
  styles: `
    .bar { padding: 0 8px; display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-sm); }
    .row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
    .sources { list-style: none; margin: 0; padding: 0; display: flex; gap: 4px; flex-wrap: wrap; align-items: center; }
    .src { max-width: 100%; }
    .src .ellipsis { max-width: 220px; }
    .tag { font-size: var(--fs-2xs); color: var(--warn-text); }
    p { margin: 0; }
    .small { font-size: var(--fs-xs); }
    .note { color: var(--muted); }
    .hint { color: var(--warn-text); }
    .err { color: var(--danger-text, var(--lvl-error)); }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); }
  `,
})
export class ChatBrowserBar {
  protected readonly store = inject(ChatBrowserStore);
  private readonly launcher = inject(BrowserLauncher);
  private readonly workspace = inject(WorkspaceStore);

  readonly sessionId = input.required<string>();

  protected readonly status = computed(() => this.store.statusOf(this.sessionId()));
  protected readonly sources = computed(() => this.store.sourcesOf(this.sessionId()));
  protected readonly selectedId = computed(() => this.workspace.selected()?.id ?? null);

  constructor() {
    effect(() => {
      const id = this.sessionId();
      untracked(() => void this.store.load(id));
    });
  }

  protected searchSelected(): void {
    const id = this.selectedId();
    if (id !== null) void this.launcher.searchEvent(id);
  }

  protected async retry(): Promise<void> {
    this.store.dismissError();
    await this.store.load(this.sessionId());
  }
}
