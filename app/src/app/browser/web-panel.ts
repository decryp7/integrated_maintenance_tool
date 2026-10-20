import { ChangeDetectionStrategy, Component, computed, inject, linkedSignal, OnInit } from '@angular/core';
import { AiStatusStore } from '../ai/ai-status.store';
import { ChatStore } from '../ai/chat/chat.store';
import { DockService } from '../dock/dock.service';
import { JobsStore } from '../jobs/jobs.store';
import { BrowserBackend } from './browser-backend';
import { Icon } from '../shared/icon';
import { BrowserStore } from './browser.store';
import { SearchEngine } from './browser.types';
import { ChatBrowserStore } from './chat-browser.store';
import { engineLabel, ENGINE_OPTIONS, noticeText } from './browser-text';

export const AI_CHAT_VIEW_ID = 'ai-chat';

/**
 * The Web panel: the controls of this window's browser (a separate window that shows only the web page),
 * "Add page to AI context" for the open chat, "Open in your browser", the browser's notices, and agent mode's
 * progress with Stop while the AI drives.
 */
@Component({
  selector: 'imt-web-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <div class="bar" role="toolbar" aria-label="Browser" i18n-aria-label="@@browser.panel.toolbar">
      <button type="button" class="icon-btn" [disabled]="!page().open" aria-label="Back" i18n-aria-label="@@browser.panel.back" title="Back" i18n-title="@@browser.panel.back.title" (click)="store.nav('back')"><imt-icon name="chevronLeft" [size]="14" /></button>
      <button type="button" class="icon-btn" [disabled]="!page().open" aria-label="Forward" i18n-aria-label="@@browser.panel.forward" title="Forward" i18n-title="@@browser.panel.forward.title" (click)="store.nav('forward')"><imt-icon name="chevronRight" [size]="14" /></button>
      @if (page().loading) {
        <button type="button" class="icon-btn" aria-label="Stop loading" i18n-aria-label="@@browser.panel.stopLoading" title="Stop loading" i18n-title="@@browser.panel.stopLoading.title" (click)="store.nav('stop')"><imt-icon name="close" [size]="14" /></button>
      } @else {
        <button type="button" class="icon-btn" [disabled]="!page().open" aria-label="Reload" i18n-aria-label="@@browser.panel.reload" title="Reload" i18n-title="@@browser.panel.reload.title" (click)="store.nav('reload')"><imt-icon name="refresh" [size]="14" /></button>
      }
      <form class="address" (submit)="$event.preventDefault(); go()">
        <label class="sr-only" for="web-address" i18n="@@browser.panel.address.label">Address or search</label>
        <input id="web-address" class="input mono" type="text" autocomplete="off" spellcheck="false" maxlength="2048" placeholder="Search or type a web address" i18n-placeholder="@@browser.panel.address.placeholder" [value]="address()" (input)="address.set(asValue($event))" data-testid="web-address" />
        <label class="sr-only" for="web-engine" i18n="@@browser.panel.engine.label">Search engine</label>
        <select id="web-engine" class="select" [value]="store.engine()" (change)="setEngine(asValue($event))" data-testid="web-engine">
          @for (e of engines(); track e) {
            <option [value]="e">{{ engineName(e) }}</option>
          }
        </select>
        <button type="submit" class="btn primary" [disabled]="!address().trim()" i18n="@@browser.panel.go">Go</button>
      </form>
    </div>

    <div class="body">
      @if (research(); as r) {
        <div class="research" role="status" data-testid="web-research">
          <span class="grow">{{ researchText(r.message) }}</span>
          <button type="button" class="btn primary" (click)="stopResearch()" data-testid="web-research-stop" i18n="@@browser.panel.researchStop">Stop</button>
        </div>
      }
      @if (page().open) {
        <div class="page" data-testid="web-page">
          <p class="title">{{ page().title || untitled }}@if (page().loading) {<span class="muted" i18n="@@browser.panel.loading"> (loading…)</span>}</p>
          <p class="url mono muted">{{ page().url }}</p>
        </div>
        <div class="actions">
          @if (ai.available()) {
            <button type="button" class="btn" [disabled]="chats.busy()" (click)="capture()" data-testid="web-capture" i18n="@@browser.panel.capture">Add page to AI context</button>
          }
          <button type="button" class="btn ghost" (click)="openExternal(page().url)" i18n="@@browser.panel.external">Open in your browser</button>
          <button type="button" class="btn ghost" (click)="store.close()" i18n="@@browser.panel.close">Close browser</button>
        </div>
      } @else {
        <p class="muted empty" i18n="@@browser.panel.empty">Search the web or open a page. It opens in a separate browser window with its own cookies and history; the AI sees a page only when you add it to a chat, or turn on read-along or agent mode in a chat.</p>
      }

      @if (driving(); as d) {
        <div class="agent" role="status" data-testid="web-agent">
          <span><ng-container i18n="@@browser.panel.agent">The AI is driving the browser</ng-container> · {{ stepText(d.stepsUsed, d.stepLimit) }}</span>
          <button type="button" class="btn primary" (click)="chats.stop(d.sessionId)" data-testid="web-agent-stop" i18n="@@browser.panel.agent.stop">Stop</button>
        </div>
      }

      @for (n of store.notices(); track n.id) {
        <div class="notice" role="status" [attr.data-kind]="n.kind">
          <p>{{ noticeLine(n.kind, n.message) }}</p>
          @if (n.url) {
            <p class="mono muted small url">{{ n.url }}</p>
          }
          <div class="row">
            @if (n.kind === 'popup' || n.kind === 'download') {
              <button type="button" class="btn" (click)="openNotice(n.id, n.url)" i18n="@@browser.panel.notice.openExternal">Open in your browser…</button>
            }
            <button type="button" class="btn ghost" (click)="store.dismiss(n.id)" i18n="@@browser.panel.notice.dismiss">Dismiss</button>
          </div>
        </div>
      }

      @if (confirming(); as c) {
        <div class="confirm" role="alertdialog" aria-labelledby="web-confirm-q" data-testid="web-confirm">
          <p id="web-confirm-q" i18n="@@browser.panel.confirm.q">Open this address in your browser?</p>
          <p class="mono small url">{{ c.url }}</p>
          <div class="row">
            <button type="button" class="btn primary" (click)="confirmExternal()" i18n="@@browser.panel.confirm.open">Open</button>
            <button type="button" class="btn ghost" (click)="confirming.set(null)" i18n="@@browser.panel.confirm.cancel">Cancel</button>
          </div>
        </div>
      }

      @if (store.error() || chats.error(); as e) {
        <p class="err" role="alert" data-testid="web-error">{{ e }}
          <button type="button" class="link" (click)="retry()" i18n="@@browser.panel.retry">Retry</button>
          <button type="button" class="link" (click)="dismissErrors()" i18n="@@browser.panel.dismiss">Dismiss</button>
        </p>
      }
      @if (capturedNote(); as note) {
        <p class="ok" role="status">{{ note }}</p>
      }
    </div>
  `,
  styles: `
    .research { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border: 1px solid var(--accent); border-radius: 6px; margin-bottom: 8px; }
    .research .grow { flex: 1; }
    :host { display: flex; flex-direction: column; height: 100%; min-height: 0; font-size: var(--fs-md); background: var(--bg-2); }
    .bar { display: flex; align-items: center; gap: 4px; padding: 6px 8px; border-bottom: 1px solid var(--border); }
    .address { display: flex; flex: 1; gap: 6px; min-width: 0; }
    .address .input { flex: 1; min-width: 80px; }
    .address .select { max-width: 150px; }
    .body { flex: 1; overflow: auto; padding: 10px 12px; display: flex; flex-direction: column; gap: 10px; }
    .page p { margin: 0; }
    .title { font-weight: 600; overflow-wrap: anywhere; }
    .url { font-size: var(--fs-xs); overflow-wrap: anywhere; }
    .actions, .row { display: flex; gap: 6px; flex-wrap: wrap; }
    .empty { margin: 0; line-height: 1.5; }
    .agent { display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 8px 10px; border: 1px solid var(--accent); border-radius: 6px; background: var(--accent-soft); }
    .notice, .confirm { padding: 8px 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-1); display: flex; flex-direction: column; gap: 6px; }
    .notice p, .confirm p { margin: 0; }
    .small { font-size: var(--fs-xs); }
    .err { color: var(--danger-text, var(--lvl-error)); margin: 0; }
    .ok { color: var(--ok); margin: 0; }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; margin-left: 6px; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); }
  `,
})
export class WebPanel implements OnInit {
  protected readonly store = inject(BrowserStore);
  protected readonly chats = inject(ChatBrowserStore);
  protected readonly ai = inject(AiStatusStore);
  private readonly chat = inject(ChatStore);
  private readonly dock = inject(DockService);
  private readonly jobs = inject(JobsStore);
  private readonly backend = inject(BrowserBackend);
  /** A research reading pages in this browser (Stop ends it). */
  protected readonly research = computed(() => this.jobs.snapshot().jobs.find((j) => j.kind === 'research' && j.state === 'running') ?? null);

  protected researchText(message: string | null): string {
    return message ? $localize`:@@browser.panel.researching:The AI is researching: ${message}:step:` : $localize`:@@browser.panel.researchingPlain:The AI is researching the web.`;
  }

  protected async stopResearch(): Promise<void> {
    await this.backend.researchStop().catch(() => 0);
  }

  protected readonly page = this.store.page;
  /** The address box follows the page shown until the user types. */
  protected readonly address = linkedSignal(() => this.store.page().url);
  protected readonly confirming = linkedSignal<string, { id: number | null; url: string } | null>({ source: () => this.store.page().url, computation: () => null });
  protected readonly capturedNote = linkedSignal<string, string | null>({ source: () => this.store.page().url, computation: () => null });
  protected readonly untitled = $localize`:@@browser.panel.untitled:Untitled page`;

  protected readonly engines = computed(() => ENGINE_OPTIONS.filter((e) => e !== 'custom' || this.store.customEngine()));
  /** The chat whose agent mode drives the browser now, if any. */
  protected readonly driving = computed(() => Object.values(this.chats.statuses()).find((s) => s.agent) ?? null);

  ngOnInit(): void {
    void this.store.load();
  }

  protected asValue(e: Event): string {
    return (e.target as HTMLInputElement | HTMLSelectElement).value;
  }

  protected engineName(e: SearchEngine): string {
    return engineLabel(e);
  }

  protected noticeLine(kind: Parameters<typeof noticeText>[0], message: string): string {
    return noticeText(kind, message);
  }

  protected stepText(used: number, limit: number): string {
    return $localize`:@@browser.panel.agent.steps:step ${used}:used: of ${limit}:limit:`;
  }

  protected setEngine(v: string): void {
    if (ENGINE_OPTIONS.includes(v as SearchEngine)) this.store.setEngine(v as SearchEngine);
  }

  async go(): Promise<void> {
    await this.store.go(this.address());
  }

  /** Hands the page to the open chat (creating one when there is none) and shows the chat. */
  async capture(): Promise<void> {
    if (!this.chat.activeId()) await this.chat.init();
    const id = this.chat.activeId();
    if (!id) return;
    if (await this.chats.capture(id)) {
      this.capturedNote.set($localize`:@@browser.panel.captured:Added to the AI chat as untrusted web content. It goes with your next question.`);
      this.dock.show(AI_CHAT_VIEW_ID);
    }
  }

  protected openExternal(url: string): void {
    this.confirming.set({ id: null, url });
  }

  protected openNotice(id: number, url: string): void {
    this.confirming.set({ id, url });
  }

  async confirmExternal(): Promise<void> {
    const c = this.confirming();
    if (!c) return;
    this.confirming.set(null);
    if ((await this.store.openExternal(c.url)) && c.id !== null) this.store.dismiss(c.id);
  }

  async retry(): Promise<void> {
    this.chats.dismissError();
    await this.store.retry();
  }

  protected dismissErrors(): void {
    this.store.dismissError();
    this.chats.dismissError();
  }
}
