import { ChangeDetectionStrategy, Component, computed, effect, inject, input, untracked } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { ChatBrowserStore } from './chat-browser.store';
import { BrowserLauncher } from './provide-browser';
import { AiLauncher } from '../ai/ai-launcher';

/**
 * The browser in the chat panel: pages added for the next question (untrusted web content), "Let AI read
 * the browser" and agent mode for this chat (both off by default, ending with the chat), the hint for small
 * models, Stop, and "Search the web for the selected event".
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
      <div class="row">
        @if (mode(); as m) {
          <span class="chip mode" [title]="m.hint" data-testid="chat-mode">{{ m.label }}</span>
        }
        <label class="toggle" title="Read-along: when you ask about it, the AI may read the page you have open in IMT's browser. It never browses by itself." i18n-title="@@browser.chatBar.readAlongTitle"><input type="checkbox" [checked]="status()?.readAlong ?? false" (change)="setReadAlong(checked($event))" data-testid="chat-read-along" /> <ng-container i18n="@@browser.chatBar.readAlong">Let AI read the browser</ng-container></label>
        <label class="toggle" title="Agent mode: the AI may drive IMT's browser step by step (open sites, click, type) for this chat; it asks before new sites, typing and forms." i18n-title="@@browser.chatBar.agentTitle"><input type="checkbox" [checked]="status()?.agent ?? false" (change)="setAgent(checked($event))" data-testid="chat-agent" /> <ng-container i18n="@@browser.chatBar.agent">Agent mode</ng-container></label>
        <button type="button" class="btn ghost" (click)="research()" data-testid="chat-research" title="The AI searches the web, reads the top results (read-only) and answers with citations" i18n-title="@@browser.chatBar.researchTitle" i18n="@@browser.chatBar.research">Research…</button>
        @if (status()?.agent) {
          <button type="button" class="btn primary" (click)="store.stop(sessionId())" data-testid="chat-agent-stop" i18n="@@browser.chatBar.stop">Stop agent</button>
        }
        <span class="spacer"></span>
        @if (selectedId() !== null) {
          <button type="button" class="btn ghost" (click)="searchSelected()" data-testid="chat-search-web" i18n="@@browser.chatBar.search">Search the web for the selected event…</button>
        }
      </div>
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
    .bar { padding: 4px 8px 0; display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-sm); }
    .row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
    .spacer { flex: 1; }
    .mode { font-size: var(--fs-2xs); padding: 0 7px; border-radius: 8px; border: 1px solid var(--accent); color: var(--text); }
    .toggle { display: inline-flex; align-items: center; gap: 4px; cursor: pointer; }
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
  private readonly ai = inject(AiLauncher);
  /** How this chat uses the web: agent mode, read-along, or research (the AI looks things up by itself). */
  protected readonly mode = computed(() => {
    const s = this.status();
    if (s?.agent) return { label: $localize`:@@browser.chatBar.mode.agent:Agent`, hint: $localize`:@@browser.chatBar.mode.agentHint:The AI drives the browser for this chat` };
    if (s?.readAlong) return { label: $localize`:@@browser.chatBar.mode.readAlong:Read-along`, hint: $localize`:@@browser.chatBar.mode.readAlongHint:The AI may read the page you have open` };
    if (s?.researchAuto) return { label: $localize`:@@browser.chatBar.mode.research:Research`, hint: $localize`:@@browser.chatBar.mode.researchHint:The AI looks things up on the web by itself, read-only, when a question needs it` };
    return null;
  });

  protected research(): void {
    void this.ai.askResearch({ sessionId: this.sessionId() });
  }
  private lastAction: (() => Promise<unknown>) | null = null;

  constructor() {
    effect(() => {
      const id = this.sessionId();
      untracked(() => void this.store.load(id));
    });
  }

  protected checked(e: Event): boolean {
    return (e.target as HTMLInputElement).checked;
  }

  protected setReadAlong(on: boolean): void {
    this.lastAction = () => this.store.setReadAlong(this.sessionId(), on);
    void this.lastAction();
  }

  protected setAgent(on: boolean): void {
    this.lastAction = () => this.store.setAgent(this.sessionId(), on);
    void this.lastAction();
  }

  protected searchSelected(): void {
    const id = this.selectedId();
    if (id !== null) void this.launcher.searchEvent(id);
  }

  protected async retry(): Promise<void> {
    this.store.dismissError();
    await (this.lastAction ?? (() => this.store.load(this.sessionId())))();
  }
}
