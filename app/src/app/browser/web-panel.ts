import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, linkedSignal, OnInit, untracked, viewChild } from '@angular/core';
import { DOCUMENT } from '@angular/common';
import { AiStatusStore } from '../ai/ai-status.store';
import { ChatStore } from '../ai/chat/chat.store';
import { DOCK_PANEL_ID } from '../dock/angular-panel';
import { DockService } from '../dock/dock.service';
import { JobsStore } from '../jobs/jobs.store';
import { Icon } from '../shared/icon';
import { AiBrowsingBanner } from './ai-browsing-banner';
import { BrowserBackend } from './browser-backend';
import { replayShortcut } from './browser-keys';
import { BrowserStore } from './browser.store';
import { PageKey, SearchEngine } from './browser.types';
import { ChatBrowserStore } from './chat-browser.store';
import { engineLabel, ENGINE_OPTIONS, noticeText } from './browser-text';
import { pageViewport } from './page-viewport';
import { WEB_VIEW } from './provide-browser';

export const AI_CHAT_VIEW_ID = 'ai-chat';

/**
 * The Browser panel (ADR 0024): the address bar and controls of this window's browser, the "AI is browsing" banner
 * with Stop, the browser's notices, and the page itself, a native webview the shell keeps over the page area (it
 * hides while the panel's tab is in the background, while an IMT overlay covers it, and while no page is open).
 * "Add page to AI context" hands the page to the open chat.
 */
@Component({
  selector: 'imt-web-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, AiBrowsingBanner],
  host: { '(keydown)': 'onKey($event)' },
  template: `
    <header class="head">
      <imt-ai-browsing-banner [progress]="progress()" />
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
          <input #address id="web-address" class="input mono" type="text" autocomplete="off" spellcheck="false" maxlength="2048" placeholder="Search or type a web address (Ctrl+L)" i18n-placeholder="@@browser.panel.address.placeholder" [value]="addressText()" (input)="addressText.set(asValue($event))" data-testid="web-address" />
          <label class="sr-only" for="web-engine" i18n="@@browser.panel.engine.label">Search engine</label>
          <select id="web-engine" class="select" [value]="store.engine()" (change)="setEngine(asValue($event))" data-testid="web-engine">
            @for (e of engines(); track e) {
              <option [value]="e">{{ engineName(e) }}</option>
            }
          </select>
          <button type="submit" class="btn primary" [disabled]="!addressText().trim()" i18n="@@browser.panel.go">Go</button>
        </form>
      </div>
      @if (page().open) {
        <div class="actions" data-testid="web-actions">
          <span class="title" [title]="page().url">{{ page().title || untitled }}@if (page().loading) {<span class="muted" i18n="@@browser.panel.loading"> (loading…)</span>}</span>
          @if (ai.available()) {
            <button type="button" class="btn" [disabled]="chats.busy()" (click)="capture()" data-testid="web-capture" i18n="@@browser.panel.capture">Add page to AI context</button>
          }
          <button type="button" class="btn ghost" (click)="openExternal(page().url)" i18n="@@browser.panel.external">Open in your browser</button>
          <button type="button" class="btn ghost" (click)="store.close()" data-testid="web-close" i18n="@@browser.panel.close">Close page</button>
        </div>
      }
      @for (n of store.notices(); track n.id) {
        <div class="strip notice" role="status" [attr.data-kind]="n.kind">
          <span class="grow">{{ noticeLine(n.kind, n.message) }}@if (n.url) {<span class="mono muted small url"> {{ n.url }}</span>}</span>
          @if (n.kind === 'popup' || n.kind === 'download') {
            <button type="button" class="btn" (click)="openNotice(n.id, n.url)" i18n="@@browser.panel.notice.openExternal">Open in your browser…</button>
          }
          <button type="button" class="btn ghost" (click)="store.dismiss(n.id)" i18n="@@browser.panel.notice.dismiss">Dismiss</button>
        </div>
      }
      @if (confirming(); as c) {
        <div class="strip confirm" role="alertdialog" aria-labelledby="web-confirm-q" data-testid="web-confirm">
          <span class="grow"><span id="web-confirm-q" i18n="@@browser.panel.confirm.q">Open this address in your browser?</span> <span class="mono small url">{{ c.url }}</span></span>
          <button type="button" class="btn primary" (click)="confirmExternal()" i18n="@@browser.panel.confirm.open">Open</button>
          <button type="button" class="btn ghost" (click)="confirming.set(null)" i18n="@@browser.panel.confirm.cancel">Cancel</button>
        </div>
      }
      @if (store.error() || chats.error(); as e) {
        <p class="strip err" role="alert" data-testid="web-error">
          <span class="grow">{{ e }}</span>
          <button type="button" class="link" (click)="retry()" i18n="@@browser.panel.retry">Retry</button>
          <button type="button" class="link" (click)="dismissErrors()" i18n="@@browser.panel.dismiss">Dismiss</button>
        </p>
      }
      @if (capturedNote(); as note) {
        <p class="strip ok" role="status">{{ note }}</p>
      }
    </header>

    <div #area class="page" tabindex="-1" role="region" aria-label="Web page" i18n-aria-label="@@browser.panel.page" [attr.data-shown]="viewport.shown()" data-testid="web-page-area">
      @if (!page().open) {
        <p class="muted placeholder" data-testid="web-empty" i18n="@@browser.panel.empty">Search the web or open a page: it shows here, with its own cookies and history. The AI sees a page only when you add it to a chat, or turn on read-along or agent mode in a chat.</p>
      } @else {
        <div class="placeholder" data-testid="web-page">
          @if (!store.docked()) {
            <p i18n="@@browser.panel.ownWindow">The page is shown in a window of its own on this system.</p>
          } @else if (viewport.covered()) {
            <p data-testid="web-page-hidden" i18n="@@browser.panel.covered">The page is hidden while a menu, dialog or message is open over it.</p>
          }
          <p class="title">{{ page().title || untitled }}</p>
          <p class="url mono muted">{{ page().url }}</p>
        </div>
      }
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; height: 100%; min-height: 0; font-size: var(--fs-md); background: var(--bg-2); }
    .head { flex: none; display: flex; flex-direction: column; border-bottom: 1px solid var(--border); }
    .bar { display: flex; align-items: center; gap: 4px; padding: 6px 8px; }
    .address { display: flex; flex: 1; gap: 6px; min-width: 0; }
    .address .input { flex: 1; min-width: 80px; }
    .address .select { max-width: 150px; }
    .actions { display: flex; align-items: center; gap: 6px; padding: 0 8px 6px; flex-wrap: wrap; }
    .actions .title { flex: 1; min-width: 0; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .strip { display: flex; align-items: center; gap: 6px; margin: 0; padding: 5px 8px; border-top: 1px solid var(--border); background: var(--bg-1); }
    .strip .grow { flex: 1; min-width: 0; overflow-wrap: anywhere; }
    .small { font-size: var(--fs-xs); }
    .err { color: var(--danger-text, var(--lvl-error)); }
    .ok { color: var(--ok); }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; margin-left: 6px; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); }
    .page { flex: 1; min-height: 0; position: relative; overflow: hidden; background: var(--bg-1); }
    .page:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
    .placeholder { margin: 0; padding: 14px 16px; line-height: 1.5; display: flex; flex-direction: column; gap: 4px; }
    .placeholder p { margin: 0; }
    .placeholder .title { font-weight: 600; overflow-wrap: anywhere; }
    .placeholder .url { font-size: var(--fs-xs); overflow-wrap: anywhere; }
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
  private readonly doc = inject(DOCUMENT);
  private readonly panelId = inject(DOCK_PANEL_ID, { optional: true }) ?? WEB_VIEW;
  private readonly addressInput = viewChild<ElementRef<HTMLInputElement>>('address');
  private readonly area = viewChild<ElementRef<HTMLElement>>('area');
  /** Keys from before the panel was there are not replayed. */
  private seenKey = untracked(() => this.store.key()?.seq ?? 0);

  protected readonly page = this.store.page;
  /** The address box follows the page shown until the user types. */
  protected readonly addressText = linkedSignal(() => this.store.page().url);
  protected readonly confirming = linkedSignal<string, { id: number | null; url: string } | null>({ source: () => this.store.page().url, computation: () => null });
  protected readonly capturedNote = linkedSignal<string, string | null>({ source: () => this.store.page().url, computation: () => null });
  protected readonly untitled = $localize`:@@browser.panel.untitled:Untitled page`;
  protected readonly engines = computed(() => ENGINE_OPTIONS.filter((e) => e !== 'custom' || this.store.customEngine()));

  /** Keeps the native page over the page area (and out of the way of IMT's overlays). */
  protected readonly viewport = pageViewport({
    area: this.area,
    open: computed(() => this.store.page().open && this.store.docked()),
    place: (p) => this.backend.place(p),
  });

  /** What the AI does now, for the banner: the agent's step, or the research job's message. */
  protected readonly progress = computed(() => {
    const d = this.store.driver();
    if (d?.kind === 'agent') {
      const s = this.chats.statuses()[d.sessionId];
      return s ? $localize`:@@browser.panel.agent.steps:step ${s.stepsUsed}:used: of ${s.stepLimit}:limit:` : null;
    }
    if (d?.kind === 'research') {
      const job = this.jobs.snapshot().jobs.find((j) => j.kind === 'research' && j.state === 'running');
      return job?.message ?? null;
    }
    return null;
  });

  constructor() {
    // The tab says it too, wherever the panel is docked.
    effect(() => {
      const title = this.store.driver() ? $localize`:@@browser.view.titleDriving:Browser · AI is browsing` : $localize`:View title@@browser.view.title:Browser`;
      untracked(() => this.dock.setTitle(this.panelId, title));
    });
    // Keys IMT took from the page.
    effect(() => {
      const k = this.store.key();
      if (!k || k.seq <= this.seenKey) return;
      this.seenKey = k.seq;
      untracked(() => this.pageKey(k.key));
    });
  }

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

  protected setEngine(v: string): void {
    if (ENGINE_OPTIONS.includes(v as SearchEngine)) this.store.setEngine(v as SearchEngine);
  }

  /** Ctrl/⌘+L in the panel focuses the address bar (in the page, the shell sends it as a `key` event). */
  protected onKey(e: KeyboardEvent): void {
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'l') {
      e.preventDefault();
      this.focusAddress();
    }
  }

  focusAddress(): void {
    const input = this.addressInput()?.nativeElement;
    input?.focus();
    input?.select();
  }

  private pageKey(k: PageKey): void {
    if (k.action === 'focusAddress') this.focusAddress();
    else if (k.action === 'returnFocus') this.area()?.nativeElement.focus();
    else replayShortcut(this.doc, k);
  }

  async go(): Promise<void> {
    await this.store.go(this.addressText());
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
