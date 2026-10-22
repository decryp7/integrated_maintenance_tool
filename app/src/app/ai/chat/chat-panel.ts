import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, signal, untracked, viewChild } from '@angular/core';
import { Platform } from '../../core/platform';
import { OverwriteGuard } from '../../ui/overwrite-guard';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { DialogService } from '../../ui/dialog.service';
import { Icon } from '../../shared/icon';
import { CdkMenu, CdkMenuItem, CdkMenuTrigger } from '@angular/cdk/menu';
import { AiBackend, toAiError } from '../ai-backend';
import { AiChatPersistence } from '../ai-chat-persistence';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';
import { AiProposalCard } from '../proposals/proposal-card';
import { AgentDecision } from '../../browser/browser.types';
import { AgentApprovalCard, AgentPaused, LinkCard, ResearchOfferCard } from '../../browser/chat-cards';
import { ChatBrowserBar } from '../../browser/chat-browser-bar';
import { ChatBrowserStore } from '../../browser/chat-browser.store';
import { ResearchOfferView, toolLabel, TurnView } from './chat-model';
import { ChatComposer } from './chat-composer';
import { ChatStore } from './chat.store';
import { JobsStore } from '../../jobs/jobs.store';
import { queuedLabel } from '../../jobs/jobs-text';
import { AiCitations } from './citation-actions';
import { isNotFound } from './markdown';
import { AiRichText } from './rich-text';

/**
 * The AI assistant chat (a dock view). Answers stream in with the tools the model used, proposals to
 * review and apply, evidence links into the grid and query bar, and the browser: pages added as untrusted
 * sources, suggested links, and agent mode's steps, approval cards and Stop.
 */
@Component({
  selector: 'imt-ai-chat-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiRichText, AiProposalCard, AgentApprovalCard, LinkCard, AgentPaused, ChatBrowserBar, ResearchOfferCard, ChatComposer, Icon, CdkMenu, CdkMenuItem, CdkMenuTrigger],
  template: `
    <div class="bar" role="toolbar" aria-label="Chat" i18n-aria-label="@@ai.chat.toolbar.label">
      <label class="sr-only" for="ai-chat-session" i18n="@@ai.chat.session.label">Chat</label>
      <select id="ai-chat-session" class="select" [value]="store.activeId() ?? ''" (change)="store.select(asValue($event))" data-testid="ai-chat-sessions">
        @for (s of store.sessions(); track s.id) {
          <option [value]="s.id">{{ sessionLabel(s) }}</option>
        }
      </select>
      <button type="button" class="icon-btn" (click)="store.newChat()" aria-label="New chat" i18n-aria-label="@@ai.chat.newChat" title="New chat" i18n-title="@@ai.chat.newChat"><imt-icon name="plus" [size]="16" /></button>
      <button type="button" class="icon-btn" [cdkMenuTriggerFor]="chatMenu" aria-label="Chat actions" i18n-aria-label="@@ai.chat.actions" title="Chat actions" i18n-title="@@ai.chat.actions" data-testid="ai-chat-actions"><imt-icon name="more" [size]="16" /></button>
    </div>
    <p class="engine muted" data-testid="ai-engine-state">{{ engineLine() }}</p>

    <ng-template #chatMenu>
      <div cdkMenu class="menu" aria-label="Chat actions" i18n-aria-label="@@ai.chat.actions">
        <button type="button" cdkMenuItem class="mi" [cdkMenuItemDisabled]="!store.activeTurns().length" (cdkMenuItemTriggered)="exportMarkdown()" i18n="@@ai.chat.export">Export…</button>
        <button type="button" cdkMenuItem class="mi danger" [cdkMenuItemDisabled]="!store.activeId()" (cdkMenuItemTriggered)="deleteChat()" i18n="@@ai.chat.deleteChat">Delete chat…</button>
      </div>
    </ng-template>

    @if (status.needsSetup()) {
      <div class="setup" role="note">
        <p i18n="@@ai.chat.setup.note">The assistant needs a local model first. It runs fully on this computer; only the model file is downloaded.</p>
        <button type="button" class="btn primary" (click)="launcher.openSetup()" i18n="@@ai.chat.setup.button">Set up AI assistant…</button>
      </div>
    }

    <div class="log" role="log" aria-live="polite" aria-label="Conversation" i18n-aria-label="@@ai.chat.log.label" #log>
      @if (!store.activeTurns().length && !store.loading()) {
        <div class="empty">
          <p class="muted" i18n="@@ai.chat.empty">Ask about the loaded logs. Answers cite events and queries you can open.</p>
          @if (store.prompts().length) {
            <ul class="starters" aria-label="Suggested questions" i18n-aria-label="@@ai.chat.starters.label">
              @for (p of store.prompts(); track p) {
                <li><button type="button" class="btn ghost starter" (click)="ask(p)">{{ p }}</button></li>
              }
            </ul>
          }
        </div>
      }
      @for (t of store.activeTurns(); track $index; let ti = $index) {
        <div class="turn">
          <p class="user imt-selectable"><span class="sr-only" i18n="@@ai.chat.you">You: </span>{{ t.user }}</p>
          @let shown = store.imagesOf(ti);
          @if (shown.length) {
            <div class="user-images" role="group" aria-label="Images with this question" i18n-aria-label="@@ai.vision.chat.turnImages" data-testid="ai-turn-images">
              @for (im of shown; track im.id) {
                <img class="thumb" [src]="im.dataUrl" [alt]="im.name" [title]="im.name" />
              }
            </div>
          }
          <div class="answer" [class.neutral]="notFound(t)" [attr.data-testid]="'ai-answer-' + ti">
            @if (t.tools.length) {
              <ul class="tools" aria-label="Assistant activity" i18n-aria-label="@@ai.chat.tools.label">
                @for (c of t.tools; track $index) {
                  <li class="tool chip" [class.running]="c.running" [class.err]="c.error" data-testid="ai-tool-chip">
                    {{ label(c.name) }}@if (c.running) {<span class="dots" aria-hidden="true">…</span>}@if (c.summary) {<span class="sum"> · {{ c.summary }}</span>}
                  </li>
                }
              </ul>
            }
            @if (t.step; as st) {
              <p class="step muted" role="status" data-testid="agent-step">{{ stepText(st.step, st.limit, st.action, st.site) }}</p>
            }
            @for (ap of t.approvals; track ap.request.requestId) {
              <imt-agent-approval [approval]="ap" (decide)="respond(ap.request.requestId, $event)" />
            }
            @for (l of t.links; track l.url) {
              <imt-link-card [link]="l" (open)="openSuggested($event)" (read)="readSuggested($event)" />
            }
            @for (o of t.offers; track o.query) {
              <imt-research-offer [offer]="o" (research)="researchOffer(o)" (agent)="agentForChat()" />
            }
            @if (t.paused; as p) {
              <imt-agent-paused [paused]="p" [canContinue]="!t.running && $last" (continue)="continueTask()" />
            }
            @if (t.text) {
              <imt-ai-rich-text class="imt-selectable" [text]="t.text" [citations]="t.citations" (record)="jump($event)" (query)="applyQuery($event)" (link)="openWeb($event.url, $event.title)" />
            } @else if (t.running && !hasPendingApproval(t)) {
              @if (queuedTurn(); as q) {
                <p class="muted thinking" role="status" data-testid="ai-chat-queued">{{ queuedText(q.position) }}</p>
              } @else {
                <p class="muted thinking" aria-label="Thinking" i18n-aria-label="@@ai.chat.thinking.label" i18n="@@ai.chat.thinking">Thinking…</p>
              }
            }
            @for (pv of t.proposals; track $index; let pi = $index) {
              <imt-ai-proposal-card [proposal]="pv.proposal" [state]="pv.state" (applied)="store.setProposalState(ti, pi, 'applied')" (dismissed)="store.setProposalState(ti, pi, 'dismissed')" />
            }
            @let sources = webSources(t);
            @if (sources.length) {
              <p class="sources muted"><ng-container i18n="@@ai.chat.sources">Web sources: </ng-container>
                @for (w of sources; track w.url) {<button type="button" class="link" (click)="openWeb(w.url, w.title)">{{ w.title }}</button>}
              </p>
            }
            @for (n of t.notes; track $index) {
              <p class="note muted imt-selectable">{{ n }}</p>
            }
            @if (t.cancelled) {
              <p class="note muted" i18n="@@ai.chat.stopped">Stopped.</p>
            }
            @if (t.error) {
              <p class="err" role="alert">{{ t.error }} <button type="button" class="link" (click)="ask(t.user)" i18n="@@ai.chat.askAgain">Ask again</button></p>
            }
          </div>
        </div>
      }
    </div>

    @if (store.error(); as e) {
      <p class="err banner" role="alert">{{ e }} <button type="button" class="link" (click)="store.dismissError()" i18n="@@ai.chat.dismiss">Dismiss</button></p>
    }
    @if (notice() ?? store.notice(); as n) {
      <p class="ok banner" role="status">{{ n }}</p>
    }

    @if (store.activeId(); as sid) {
      <imt-chat-browser-bar [sessionId]="sid" />
    }

    <imt-chat-composer [sessionId]="store.activeId()" />
  `,
  styles: `
    :host { display: flex; flex-direction: column; height: 100%; min-height: 0; font-size: var(--fs-md); }
    .bar { display: flex; align-items: center; gap: 4px; padding: 6px 8px 2px; flex-wrap: wrap; }
    .bar .select { flex: 1; min-width: 0; max-width: 320px; }
    .engine { margin: 0; padding: 0 8px 4px; font-size: var(--fs-xs); border-bottom: 1px solid var(--border); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .menu { min-width: 180px; display: flex; flex-direction: column; padding: 4px; border-radius: 8px; background: var(--bg-2); border: 1px solid var(--border-3); box-shadow: var(--shadow); }
    .menu:focus { outline: none; }
    .mi { display: flex; align-items: center; min-height: 28px; padding: 0 10px; border: 0; border-radius: 5px; background: transparent; color: var(--text-2); cursor: pointer; text-align: start; font: inherit; }
    .mi:hover:not([aria-disabled='true']), .mi:focus { background: var(--bg-active); color: var(--text); outline: none; }
    .mi:focus-visible { box-shadow: inset 0 0 0 1px var(--accent); }
    .mi[aria-disabled='true'] { color: var(--muted); cursor: default; }
    .mi.danger:not([aria-disabled='true']) { color: var(--danger-text, var(--lvl-error)); }
    .setup { padding: 10px 12px; border-bottom: 1px solid var(--border); background: var(--bg-1); }
    .setup p { margin: 0 0 8px; }
    /* Positioned, so the text for screen readers in it stays in the list (else the dock scrolled past the panel). */
    .log { position: relative; flex: 1; overflow-y: auto; padding: 10px 12px; display: flex; flex-direction: column; gap: 14px; min-height: 0; }
    .empty .starters { list-style: none; padding: 0; margin: 8px 0 0; display: flex; flex-direction: column; gap: 6px; }
    .starter { text-align: left; white-space: normal; }
    .turn { display: flex; flex-direction: column; gap: 6px; }
    .user { align-self: flex-end; max-width: 85%; margin: 0; background: var(--accent-soft); padding: 6px 10px; border-radius: 10px 10px 2px 10px; white-space: pre-wrap; overflow-wrap: anywhere; }
    .answer { display: flex; flex-direction: column; gap: 8px; }
    .answer.neutral imt-ai-rich-text { color: var(--muted); font-style: italic; }
    .tools { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: 4px; }
    .tool { font-size: var(--fs-xs); }
    .tool.running { opacity: 0.8; }
    .tool.err { border-color: var(--lvl-error); }
    .sum { color: var(--text-2); }
    .thinking { margin: 0; }
    .sources, .note, .step { margin: 0; font-size: var(--fs-sm); }
    .sources .link { margin-left: 6px; }
    .err { color: var(--danger-text, var(--lvl-error)); margin: 0; }
    .ok { color: var(--ok); margin: 0; }
    .banner { padding: 4px 12px; font-size: var(--fs-sm); }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); }
    .thumb { width: 40px; height: 28px; object-fit: cover; border-radius: 3px; border: 1px solid var(--border); background: var(--bg-1); }
    .user-images { align-self: flex-end; display: flex; gap: 4px; }
  `,
})
export class AiChatPanel {
  protected readonly store = inject(ChatStore);
  private readonly jobs = inject(JobsStore);
  /** The active chat's turn while it waits in the AI queue behind other requests. */
  protected readonly queuedTurn = computed(() => {
    const id = this.store.activeId();
    return id ? this.jobs.queuedTurn(id) : null;
  });

  protected queuedText(position: number | null): string {
    return `${queuedLabel(position)} · ${$localize`:@@ai.chat.queuedHint:the AI answers one request at a time`}`;
  }
  protected readonly status = inject(AiStatusStore);
  protected readonly launcher = inject(AiLauncher);
  private readonly citations = inject(AiCitations);
  private readonly backend = inject(AiBackend);
  private readonly platform = inject(Platform);
  private readonly overwrite = inject(OverwriteGuard);
  private readonly dialogs = inject(DialogService);
  private readonly persistence = inject(AiChatPersistence);
  private readonly browser = inject(ChatBrowserStore);
  private readonly logEl = viewChild<ElementRef<HTMLElement>>('log');

  protected readonly notice = signal<string | null>(null);
  protected readonly engineLine = computed(() => {
    const s = this.status.status();
    if (!s) return '';
    if (!s.available) return $localize`:@@ai.chat.engine.unavailable:AI unavailable`;
    const model = s.model ? s.model.replace(/\.gguf$/i, '') : $localize`:@@ai.chat.engine.noModel:no model`;
    return `${model} · ${s.state}${s.backend ? ` · ${s.backend}` : ''}`;
  });

  constructor() {
    void this.store.init();
    let first = true;
    effect(() => {
      this.persistence.restored();
      if (first) {
        first = false;
        return;
      }
      untracked(() => void this.store.reload());
    });
    // Keep the newest message in view while answers stream.
    effect(() => {
      this.store.activeTurns();
      const el = this.logEl()?.nativeElement;
      if (el) queueMicrotask(() => (el.scrollTop = el.scrollHeight));
    });
  }

  protected asValue(e: Event): string {
    return (e.target as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement).value;
  }

  protected sessionLabel(s: { title: string; running: boolean }): string {
    const title = s.title || $localize`:@@ai.chat.session.untitled:New chat`;
    return s.running ? $localize`:@@ai.chat.session.answering:${title}:title: (answering…)` : title;
  }

  protected label(name: string): string {
    return toolLabel(name);
  }

  protected notFound(t: TurnView): boolean {
    return !t.running && isNotFound(t.text);
  }

  protected webSources(t: TurnView): { url: string; title: string }[] {
    return t.running ? [] : t.citations.flatMap((c) => (c.kind === 'web' ? [{ url: c.url, title: c.title }] : []));
  }

  protected hasPendingApproval(t: TurnView): boolean {
    return t.approvals.some((a) => a.state === 'pending');
  }

  protected stepText(step: number, limit: number, action: string, site: string | null): string {
    const where = site ?? '';
    return $localize`:@@ai.chat.agentStep:Step ${step}:step: of ${limit}:limit:: ${toolLabel('browser_' + action)}:action: ${where}:site:…`;
  }

  protected async respond(requestId: number, d: AgentDecision): Promise<void> {
    const id = this.store.activeId();
    if (!id) return;
    this.store.setApprovalState(requestId, 'sending');
    const delivered = await this.browser.respond(id, requestId, d);
    this.store.setApprovalState(requestId, delivered ? d : 'expired');
  }

  /** "Open and let AI read it": the page joins the chat as untrusted content and the AI goes on with it. */
  protected async readSuggested(url: string): Promise<void> {
    const id = this.store.activeId();
    if (id) await this.browser.openAndRead(id, url);
  }

  /** "Research this" on an offer card (the engine asks to confirm a search text with workspace data in it). */
  protected async researchOffer(o: ResearchOfferView): Promise<void> {
    const id = this.store.activeId();
    await this.launcher.research({ question: o.question, query: o.query, sessionId: id });
  }

  /** "Let the AI drive the browser": agent mode for this chat only. */
  protected async agentForChat(): Promise<void> {
    const id = this.store.activeId();
    if (id) await this.browser.setAgent(id, true);
  }

  protected async openSuggested(url: string): Promise<void> {
    const id = this.store.activeId();
    if (id) await this.browser.openSuggested(id, url);
  }

  /** "Continue" after the step cap: another batch of steps, and the task goes on. */
  protected async continueTask(): Promise<void> {
    const id = this.store.activeId();
    if (!id) return;
    if (await this.browser.continueTask(id)) await this.store.send($localize`:@@ai.chat.continueTask:Continue.`);
  }

  async ask(question: string): Promise<void> {
    await this.store.send(question);
  }

  protected async jump(id: number): Promise<void> {
    await this.citations.record(id);
  }

  protected async applyQuery(ql: string): Promise<void> {
    const err = await this.citations.query(ql);
    this.notice.set(
      err ? $localize`:@@ai.chat.notice.notApplied:Not applied: ${err}:error:` : $localize`:@@ai.chat.notice.applied:Applied ${ql}:query:`,
    );
  }

  protected async openWeb(url: string, title: string): Promise<void> {
    const err = await this.citations.web(url, title);
    if (err) this.notice.set(err);
  }

  /** "Delete chat…": asks first (the chat and its answers cannot be brought back). */
  async deleteChat(): Promise<void> {
    const id = this.store.activeId();
    if (!id) return;
    const title = this.store.active()?.title || $localize`:@@ai.chat.session.untitled:New chat`;
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:@@ai.chat.delete.title:Delete this chat?`,
      width: 420,
      inputs: {
        message: $localize`:@@ai.chat.delete.message:"${title}:title:" and its answers are deleted. This cannot be undone.`,
        confirmLabel: $localize`:@@ai.chat.delete.confirm:Delete chat`,
        danger: true,
      },
    });
    if ((await ref.closed) === true) await this.store.deleteChat(id);
  }

  async exportMarkdown(): Promise<void> {
    const id = this.store.activeId();
    if (!id) return;
    const name = (this.store.active()?.title || 'chat').replace(/[^\w.-]+/g, '-').slice(0, 60) || 'chat';
    const path = await this.platform.pickSavePath($localize`:@@ai.chat.export.dialogTitle:Export chat`, `${name}.md`, [{ name: 'Markdown', extensions: ['md'] }]);
    if (!path) return;
    try {
      const written = await this.overwrite.write(path, (o) => this.backend.chatExportMarkdown(id, path, o));
      if (written !== null) this.notice.set($localize`:@@ai.chat.notice.exported:Exported to ${written}:path:`);
    } catch (e) {
      this.notice.set($localize`:@@ai.chat.notice.notExported:Not exported: ${toAiError(e).message}:error:`);

    }
  }
}
