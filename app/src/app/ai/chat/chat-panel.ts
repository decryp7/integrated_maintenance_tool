import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, signal, untracked, viewChild } from '@angular/core';
import { formatTime } from '../../core/format';
import { Platform } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
import { OverwriteGuard } from '../../ui/overwrite-guard';
import { AiBackend, toAiError } from '../ai-backend';
import { AiChatPersistence } from '../ai-chat-persistence';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';
import { AiProposalCard } from '../proposals/proposal-card';
import { AgentDecision } from '../../browser/browser.types';
import { AgentApprovalCard, AgentPaused, LinkCard, ResearchOfferCard } from '../../browser/chat-cards';
import { ChatBrowserBar } from '../../browser/chat-browser-bar';
import { ChatBrowserStore } from '../../browser/chat-browser.store';
import { describeContext, ResearchOfferView, toolLabel, TurnView } from './chat-model';
import { ChatStore } from './chat.store';
import { JobsStore } from '../../jobs/jobs.store';
import { queuedLabel } from '../../jobs/jobs-text';
import { AiCitations } from './citation-actions';
import { isNotFound } from './markdown';
import { AiRichText } from './rich-text';
import { Icon } from '../../shared/icon';
import { AiImage } from '../ai.types';
import { AiImageStore } from '../vision/ai-image.store';
import { dragHasFiles, imageFilesOf, imageFilter, MAX_IMAGES_PER_QUESTION } from '../vision/image-files';
import { AiNoVisionCard } from '../vision/no-vision-card';
import { DropTargets } from '../../core/dropped-files';

/**
 * The AI assistant chat (a dock view). Answers stream in with the tools the model used, proposals to
 * review and apply, evidence links into the grid and query bar, and the browser: pages added as untrusted
 * sources, suggested links, and agent mode's steps, approval cards and Stop.
 */
@Component({
  selector: 'imt-ai-chat-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiRichText, AiProposalCard, AgentApprovalCard, LinkCard, AgentPaused, ChatBrowserBar, ResearchOfferCard, Icon, AiNoVisionCard],
  template: `
    <div class="bar" role="toolbar" aria-label="Chat" i18n-aria-label="@@ai.chat.toolbar.label">
      <label class="sr-only" for="ai-chat-session" i18n="@@ai.chat.session.label">Chat</label>
      <select id="ai-chat-session" class="select" [value]="store.activeId() ?? ''" (change)="store.select(asValue($event))" data-testid="ai-chat-sessions">
        @for (s of store.sessions(); track s.id) {
          <option [value]="s.id">{{ sessionLabel(s) }}</option>
        }
      </select>
      <button type="button" class="btn ghost" (click)="store.newChat()" i18n="@@ai.chat.newChat">New chat</button>
      <button type="button" class="btn ghost" [disabled]="!store.activeTurns().length" (click)="exportMarkdown()" i18n="@@ai.chat.export">Export…</button>
      <button type="button" class="btn ghost" [disabled]="!store.activeId()" aria-label="Delete this chat" i18n-aria-label="@@ai.chat.delete.label" title="Delete this chat" i18n-title="@@ai.chat.delete.title" (click)="deleteChat()" i18n="@@ai.chat.delete">Delete</button>
      <span class="spacer"></span>
      <span class="engine muted" data-testid="ai-engine-state">{{ engineLine() }}</span>
    </div>

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
                    {{ label(c.name) }}@if (c.running) {<span class="dots" aria-hidden="true">…</span>}@if (c.summary) {<span class="sum">: {{ c.summary }}</span>}
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

    <div class="context">
      <button type="button" class="chip ctx" [attr.aria-expanded]="contextOpen()" aria-controls="ai-chat-context" (click)="contextOpen.set(!contextOpen())" data-testid="ai-context-chip" i18n="@@ai.chat.context.chip">
        Context: {{ contextSummary() }}
      </button>
      @if (contextOpen()) {
        <fieldset id="ai-chat-context" class="ctx-list">
          <legend class="sr-only" i18n="@@ai.chat.context.legend">What is sent with your question</legend>
          @for (c of contextLines(); track c.key) {
            <label class="ctx-item" [class.off]="!c.available">
              <input type="checkbox" [checked]="store.toggles()[c.key] && c.available" [disabled]="!c.available" (change)="store.toggle(c.key)" />
              {{ c.label }}
            </label>
          }
          <p class="muted small" i18n="@@ai.chat.context.note">The engine checks these and reads the events itself. Nothing leaves this computer.</p>
        </fieldset>
      }
    </div>

    @if (images.noVision()) {
      <imt-ai-no-vision class="no-vision" />
    }
    @if (attachError(); as e) {
      <p class="err banner" role="alert" data-testid="ai-attach-error">{{ e }} <button type="button" class="link" (click)="retryAttach()" i18n="@@ai.vision.retry">Retry</button> <button type="button" class="link" (click)="attachError.set(null)" i18n="@@ai.vision.dismiss">Dismiss</button></p>
    }
    @if (store.attachments().length) {
      <ul class="attachments" aria-label="Attached images" i18n-aria-label="@@ai.vision.chat.attachments">
        @for (a of store.attachments(); track a.id) {
          <li class="attachment chip" data-testid="ai-attachment">
            <img class="thumb" [src]="a.dataUrl" [alt]="a.name" />
            <span class="att-name" [title]="a.name">{{ a.name }}</span>
            <button type="button" class="link small" [disabled]="store.running() || !status.hasModel()" (click)="store.sendRelated(a)" i18n="@@ai.vision.chat.related">Find related logs</button>
            <button type="button" class="icon-btn" [attr.aria-label]="removeLabel(a.name)" [attr.title]="removeLabel(a.name)" (click)="store.detach(a.id)"><imt-icon name="close" [size]="12" /></button>
          </li>
        }
      </ul>
    }

    <form #composer class="composer" [class.dropping]="dropping()" (submit)="$event.preventDefault(); submit()" (dragover)="onDragOver($event)" (dragleave)="dropping.set(false)" (drop)="onDrop($event)">
      <button
        type="button"
        class="icon-btn attach"
        aria-label="Attach an image"
        i18n-aria-label="@@ai.vision.chat.attach"
        title="Attach an image (or paste or drop one)"
        i18n-title="@@ai.vision.chat.attachTitle"
        data-testid="ai-attach"
        [disabled]="!status.hasModel() || images.opening() || store.attachments().length >= maxImages"
        (click)="attachFile()"
      ><imt-icon name="image" [size]="16" /></button>
      <label class="sr-only" for="ai-chat-input" i18n="@@ai.chat.input.label">Ask about the logs</label>
      <textarea
        id="ai-chat-input"
        class="input"
        rows="2"
        maxlength="8000"
        placeholder="Ask about the logs…"
        i18n-placeholder="@@ai.chat.input.placeholder"
        [value]="draft()"
        [disabled]="!status.hasModel()"
        (input)="draft.set(asValue($event))"
        (keydown.enter)="onEnter($event)"
        (paste)="onPaste($event)"
      ></textarea>
      @if (store.running()) {
        <button type="button" class="btn" (click)="store.cancel()" i18n="@@ai.chat.stop">Stop</button>
      } @else {
        <button type="submit" class="btn primary" [disabled]="!draft().trim() || !status.hasModel()" i18n="@@ai.chat.send">Send</button>
      }
    </form>
  `,
  styles: `
    :host { display: flex; flex-direction: column; height: 100%; min-height: 0; font-size: var(--fs-md); }
    .bar { display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
    .bar .select { max-width: 220px; }
    .spacer { flex: 1; }
    .engine { font-size: var(--fs-xs); }
    .setup { padding: 10px 12px; border-bottom: 1px solid var(--border); background: var(--bg-1); }
    .setup p { margin: 0 0 8px; }
    .log { flex: 1; overflow-y: auto; padding: 10px 12px; display: flex; flex-direction: column; gap: 14px; min-height: 0; }
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
    .context { padding: 4px 8px 0; }
    .ctx { cursor: pointer; font-size: var(--fs-xs); max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .ctx-list { border: 1px solid var(--border); border-radius: 6px; margin: 4px 0 0; padding: 6px 8px; display: flex; flex-direction: column; gap: 2px; font-size: var(--fs-sm); }
    .ctx-item.off { color: var(--muted); }
    .small { margin: 4px 0 0; font-size: var(--fs-xs); }
    .composer { display: flex; gap: 6px; padding: 8px; align-items: flex-end; }
    .composer textarea { flex: 1; resize: vertical; min-height: 38px; font-family: inherit; }
    .composer.dropping { outline: 2px dashed var(--accent); outline-offset: -4px; }
    .attach { align-self: center; }
    .attachments { list-style: none; margin: 0; padding: 6px 8px 0; display: flex; flex-wrap: wrap; gap: 6px; }
    .attachment { display: flex; align-items: center; gap: 6px; max-width: 100%; font-size: var(--fs-xs); }
    .att-name { max-width: 16ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .thumb { width: 40px; height: 28px; object-fit: cover; border-radius: 3px; border: 1px solid var(--border); background: var(--bg-1); }
    .user-images { align-self: flex-end; display: flex; gap: 4px; }
    .no-vision { display: block; margin: 6px 8px 0; }
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
  private readonly persistence = inject(AiChatPersistence);
  private readonly workspace = inject(WorkspaceStore);
  private readonly browser = inject(ChatBrowserStore);
  private readonly logEl = viewChild<ElementRef<HTMLElement>>('log');
  private readonly composerEl = viewChild<ElementRef<HTMLElement>>('composer');
  private readonly dropTargets = inject(DropTargets);

  protected readonly images = inject(AiImageStore);
  protected readonly maxImages = MAX_IMAGES_PER_QUESTION;
  /** Why the last attach failed (Retry repeats it). */
  protected readonly attachError = signal<string | null>(null);
  protected readonly dropping = signal(false);
  private lastAttach: (() => Promise<void>) | null = null;

  protected readonly draft = signal('');
  protected readonly contextOpen = signal(false);
  protected readonly notice = signal<string | null>(null);

  protected readonly contextLines = computed(() => describeContext(this.store.sources(), (ms) => formatTime(ms, this.workspace.timezone())));
  protected readonly contextSummary = computed(() => {
    const on = this.store.toggles();
    const parts = this.contextLines()
      .filter((c) => c.available && on[c.key])
      .map((c) => (c.label.split(':')[0] ?? '').toLowerCase());
    return parts.length ? parts.join(', ') : $localize`:@@ai.chat.context.none:nothing selected`;
  });
  protected readonly engineLine = computed(() => {
    const s = this.status.status();
    if (!s) return '';
    if (!s.available) return $localize`:@@ai.chat.engine.unavailable:AI unavailable`;
    const model = s.model ? s.model.replace(/\.gguf$/i, '') : $localize`:@@ai.chat.engine.noModel:no model`;
    return `${model} · ${s.state}${s.backend ? ` · ${s.backend}` : ''}`;
  });

  constructor() {
    // The composer takes images dropped from the desktop (the shell routes OS drops by target, ADR 0025).
    effect((onCleanup) => {
      const el = this.composerEl()?.nativeElement;
      if (!el) return;
      onCleanup(this.dropTargets.register({ kind: 'aiImages', element: el, accept: (paths) => this.acceptDroppedPaths(paths) }));
    });
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

  protected onEnter(e: Event): void {
    const k = e as KeyboardEvent;
    if (k.shiftKey || k.isComposing) return;
    k.preventDefault();
    void this.submit();
  }

  protected removeLabel(name: string): string {
    return $localize`:@@ai.vision.chat.remove:Remove ${name}:name:`;
  }

  /** A text-only model is not given images: the no-vision message says what to do instead. */
  private canAttach(): boolean {
    if (this.status.status()?.vision === false) {
      this.images.refuseNoVision();
      return false;
    }
    return true;
  }

  /** Opens an image through the engine (it decodes it) and attaches the clean copy. */
  private async attachVia(open: () => Promise<AiImage | null>): Promise<void> {
    const run = async () => {
      this.attachError.set(null);
      const img = await open();
      if (img) {
        this.store.attach(img);
        return;
      }
      // Shown here, with its own Retry, rather than in the image view.
      this.attachError.set(this.images.openError());
      this.images.dismissOpenError();
    };
    this.lastAttach = run;
    await run();
  }

  protected async retryAttach(): Promise<void> {
    await this.lastAttach?.();
  }

  /** The attach button: the platform's file picker (Rust reads the path the user chose). */
  async attachFile(): Promise<void> {
    if (!this.canAttach()) return;
    const path = await this.platform.pickOpenPath($localize`:@@ai.vision.chat.pickTitle:Attach an image`, imageFilter());
    if (path) await this.attachVia(() => this.images.openPath(path, false));
  }

  /**
   * Files dropped from the desktop onto the composer (the OS drop, routed here by the shell as an `aiImages` target):
   * each is opened by the engine, which recognises images by their content; what it refuses is not claimed and opens
   * as logs. With a text-only model the drop is claimed and the offer to add image support is shown.
   */
  async acceptDroppedPaths(paths: readonly string[]): Promise<readonly string[]> {
    if (!this.canAttach()) return paths;
    const room = Math.max(0, MAX_IMAGES_PER_QUESTION - this.store.attachments().length);
    const claimed: string[] = [];
    for (const path of paths.slice(0, room)) {
      const img = await this.images.openPath(path, false);
      if (img) {
        this.store.attach(img);
        claimed.push(path);
      } else {
        this.images.dismissOpenError();
      }
    }
    if (paths.length > room) this.store.setNotice($localize`:@@ai.vision.chat.tooMany:At most ${MAX_IMAGES_PER_QUESTION}:max: images go with one question.`);
    return claimed;
  }

  /** Pasted or dropped image files: their bytes go to the engine as they are (never decoded here). */
  private async attachFiles(files: File[]): Promise<void> {
    if (!this.canAttach()) return;
    const room = Math.max(0, MAX_IMAGES_PER_QUESTION - this.store.attachments().length);
    if (files.length > room) this.store.setNotice($localize`:@@ai.vision.chat.tooMany:At most ${MAX_IMAGES_PER_QUESTION}:max: images go with one question.`);
    for (const f of files.slice(0, room)) await this.attachVia(() => this.images.pasteFile(f, false));
  }

  protected onPaste(e: ClipboardEvent): void {
    const files = imageFilesOf(e.clipboardData);
    if (!files.length) return; // text: the textarea pastes it
    e.preventDefault();
    void this.attachFiles(files);
  }

  protected onDragOver(e: DragEvent): void {
    if (!dragHasFiles(e.dataTransfer)) return;
    e.preventDefault();
    this.dropping.set(true);
  }

  protected onDrop(e: DragEvent): void {
    this.dropping.set(false);
    if (!dragHasFiles(e.dataTransfer)) return;
    // Never let the browser open a dropped file.
    e.preventDefault();
    e.stopPropagation();
    const files = imageFilesOf(e.dataTransfer);
    if (!files.length) {
      this.store.setNotice($localize`:@@ai.vision.chat.notImage:Only PNG, JPEG, BMP, GIF and WebP images can be attached.`);
      return;
    }
    void this.attachFiles(files);
  }

  async submit(): Promise<void> {
    if (await this.store.send(this.draft())) this.draft.set('');
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

  async deleteChat(): Promise<void> {
    const id = this.store.activeId();
    if (id) await this.store.deleteChat(id);
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
