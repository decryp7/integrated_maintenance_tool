import { CdkMenu, CdkMenuItemRadio, CdkMenuTrigger } from '@angular/cdk/menu';
import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, input, signal, viewChild } from '@angular/core';
import { formatTime } from '../../core/format';
import { Platform } from '../../core/platform';
import { DropTargets } from '../../core/dropped-files';
import { WorkspaceStore } from '../../core/workspace.store';
import { ChatBrowserMode, chatBrowserMode } from '../../browser/browser.types';
import { ChatBrowserStore } from '../../browser/chat-browser.store';
import { Icon } from '../../shared/icon';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';
import { AiImage } from '../ai.types';
import { AiImageStore } from '../vision/ai-image.store';
import { AiNoVisionCard } from '../vision/no-vision-card';
import { dragHasFiles, imageFilesOf, imageFilter, MAX_IMAGES_PER_QUESTION } from '../vision/image-files';
import { ContextMode, contextModeLabel, describeContext } from './chat-model';
import { ChatStore } from './chat.store';

/** Lines the question box grows to before it scrolls. */
export const COMPOSER_MAX_LINES = 8;

/** The height the question box takes for its content: its scroll height, at most `maxLines` lines. */
export function composerHeight(scrollHeight: number, lineHeight: number, paddingY: number, maxLines = COMPOSER_MAX_LINES): number {
  return Math.min(scrollHeight, lineHeight * maxLines + paddingY);
}

const BROWSER_MODES: readonly ChatBrowserMode[] = ['off', 'readAlong', 'agent'];
const CONTEXT_MODES: readonly ContextMode[] = ['selection', 'view', 'none'];

/**
 * The chat's composer: one card with the attached images (thumbnail chips), the question box (grows to 8 lines, then
 * scrolls; Enter sends, Shift+Enter starts a new line) and a toolbar: attach an image, "Context" (what goes with the
 * question), "Browser" (Off, read-along or agent mode for this chat), "Research…" and Send, which becomes Stop while
 * the AI answers. The whole card takes dropped and pasted images. State lives in `ChatStore` and `ChatBrowserStore`.
 */
@Component({
  selector: 'imt-chat-composer',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, CdkMenu, CdkMenuItemRadio, CdkMenuTrigger, AiNoVisionCard],
  template: `
    @if (images.noVision()) {
      <imt-ai-no-vision class="no-vision" />
    }
    @if (attachError(); as e) {
      <p class="err" role="alert" data-testid="ai-attach-error">{{ e }} <button type="button" class="link" (click)="retryAttach()" i18n="@@ai.vision.retry">Retry</button> <button type="button" class="link" (click)="attachError.set(null)" i18n="@@ai.vision.dismiss">Dismiss</button></p>
    }
    <form #card class="card" [class.dropping]="dropping()" (submit)="$event.preventDefault(); submit()" (dragover)="onDragOver($event)" (dragleave)="dropping.set(false)" (drop)="onDrop($event)" data-testid="ai-composer">
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
      <label class="sr-only" for="ai-chat-input" i18n="@@ai.chat.input.label">Ask about the logs</label>
      <textarea
        #input
        id="ai-chat-input"
        class="input"
        rows="1"
        maxlength="8000"
        placeholder="Ask about the logs…"
        i18n-placeholder="@@ai.chat.input.placeholder"
        aria-describedby="ai-chat-hint"
        [value]="draft()"
        [disabled]="!status.hasModel()"
        (input)="onInput($event)"
        (keydown.enter)="onEnter($event)"
        (keydown.escape)="onEscape($event)"
        (paste)="onPaste($event)"
      ></textarea>
      <div class="tools" role="toolbar" aria-label="Question options" i18n-aria-label="@@ai.chat.composer.toolbar">
        <button type="button" class="icon-btn" data-testid="ai-attach" [attr.aria-label]="attachLabel" [title]="attachTitle()" [disabled]="!!attachBlocked()" (click)="attachFile()">
          <imt-icon name="image" [size]="16" />
        </button>
        <button type="button" class="btn ghost menu-btn" data-testid="ai-context-menu" [cdkMenuTriggerFor]="contextMenu" [title]="contextTitle()">
          <span class="ellipsis">{{ contextButton() }}</span><imt-icon name="chevronDown" [size]="12" />
        </button>
        @if (sessionId()) {
          <button type="button" class="btn ghost menu-btn" data-testid="ai-browser-menu" [cdkMenuTriggerFor]="browserMenu" [title]="browserHint(browserMode())">
            <span class="ellipsis">{{ browserButton() }}</span><imt-icon name="chevronDown" [size]="12" />
          </button>
        }
        <button type="button" class="btn ghost" data-testid="chat-research" [title]="researchTitle()" (click)="research()" i18n="@@browser.chatBar.research">Research…</button>
        <span class="spacer"></span>
        @if (store.running()) {
          <button type="button" class="btn" data-testid="ai-stop" title="Stop the answer (Esc)" i18n-title="@@ai.chat.stopTitle" (click)="stop()" i18n="@@ai.chat.stop">Stop</button>
        } @else {
          <button type="submit" class="btn primary" data-testid="ai-send" [disabled]="!!sendBlocked()" [title]="sendBlocked() ?? sendTitle" i18n="@@ai.chat.send">Send</button>
        }
      </div>
    </form>
    <p id="ai-chat-hint" class="hint muted" i18n="@@ai.chat.composer.hint">Enter to send · Shift+Enter for a new line</p>

    <ng-template #contextMenu>
      <div cdkMenu class="menu" aria-label="Context sent with your question" i18n-aria-label="@@ai.chat.context.menu" data-testid="ai-context-list">
        @for (m of contextModes; track m) {
          <button
            type="button"
            cdkMenuItemRadio
            class="mi"
            [cdkMenuItemChecked]="store.contextMode() === m"
            [cdkMenuItemDisabled]="m === 'selection' && !selectedCount()"
            [attr.title]="m === 'selection' && !selectedCount() ? noSelection : null"
            (cdkMenuItemTriggered)="store.setContextMode(m)"
          >
            <span class="check" aria-hidden="true">{{ store.contextMode() === m ? '●' : '' }}</span>
            <span class="mi-text">
              <span class="mi-label">{{ contextLabel(m) }}</span>
              @if (contextDetail(m); as d) {
                <span class="desc">{{ d }}</span>
              }
            </span>
          </button>
        }
        <p class="foot" i18n="@@ai.chat.context.note">The engine checks these and reads the events itself. Nothing leaves this computer.</p>
      </div>
    </ng-template>

    <ng-template #browserMenu>
      <div cdkMenu class="menu" aria-label="Browser for this chat" i18n-aria-label="@@ai.chat.browser.menu" data-testid="ai-browser-list">
        @for (m of browserModes; track m) {
          <button type="button" cdkMenuItemRadio class="mi" [cdkMenuItemChecked]="browserMode() === m" (cdkMenuItemTriggered)="setBrowser(m)" [attr.data-testid]="'ai-browser-' + m">
            <span class="check" aria-hidden="true">{{ browserMode() === m ? '●' : '' }}</span>
            <span class="mi-text">
              <span class="mi-label">{{ browserLabel(m) }}</span>
              <span class="desc">{{ browserHint(m) }}</span>
              @if (m === 'agent' && browserStatus()?.smallModel) {
                <span class="desc warn" i18n="@@browser.chatBar.smallModel">Agent mode works best with a 7B or larger model; yours may lose track on long tasks.</span>
              }
            </span>
          </button>
        }
      </div>
    </ng-template>
  `,
  styles: `
    :host { display: block; padding: 6px 8px 4px; container-type: inline-size; }
    .card { display: flex; flex-direction: column; gap: 4px; padding: 6px 6px 4px; border: 1px solid var(--border-3); border-radius: 8px; background: var(--bg-1); }
    .card:focus-within { border-color: var(--accent); box-shadow: 0 0 0 1px var(--accent); }
    .card.dropping { outline: 2px dashed var(--accent); outline-offset: 2px; }
    textarea { resize: none; border: 0; background: transparent; padding: 4px; font-family: inherit; font-size: inherit; line-height: 1.4; color: inherit; overflow-y: auto; min-height: 0; }
    textarea:focus { outline: none; }
    .tools { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; }
    .spacer { flex: 1; }
    .menu-btn { display: inline-flex; align-items: center; gap: 4px; max-width: 220px; }
    .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
    .attachments { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: 6px; }
    .attachment { display: flex; align-items: center; gap: 6px; max-width: 100%; font-size: var(--fs-xs); }
    .att-name { max-width: 16ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .thumb { width: 40px; height: 28px; object-fit: cover; border-radius: 3px; border: 1px solid var(--border); background: var(--bg-1); }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); }
    .link[disabled] { cursor: default; color: var(--muted); }
    .small { font-size: var(--fs-xs); }
    .hint { margin: 2px 4px 0; font-size: var(--fs-xs); }
    .no-vision { display: block; margin-bottom: 6px; }
    .err { color: var(--danger-text, var(--lvl-error)); margin: 0 0 6px; font-size: var(--fs-sm); }
    @container (max-width: 320px) { .hint { display: none; } }
    .menu { min-width: 240px; max-width: min(360px, calc(100vw - 16px)); display: flex; flex-direction: column; padding: 4px; border-radius: 8px; background: var(--bg-2); border: 1px solid var(--border-3); box-shadow: var(--shadow); }
    .menu:focus { outline: none; }
    .mi { display: flex; align-items: flex-start; gap: 6px; padding: 5px 10px 5px 6px; border: 0; border-radius: 5px; background: transparent; color: var(--text); cursor: pointer; text-align: start; font: inherit; }
    .mi:hover:not([aria-disabled='true']), .mi:focus { background: var(--bg-active); outline: none; }
    .mi:focus-visible { box-shadow: inset 0 0 0 1px var(--accent); }
    .mi[aria-disabled='true'] { color: var(--muted); cursor: default; }
    .check { width: 12px; flex: none; color: var(--accent-text); font-size: var(--fs-2xs); line-height: 1.6; }
    .mi-text { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
    .desc { font-size: var(--fs-xs); color: var(--muted); white-space: normal; overflow-wrap: anywhere; }
    .desc.warn { color: var(--warn-text); }
    .foot { margin: 4px 6px 2px; padding-top: 4px; border-top: 1px solid var(--border); font-size: var(--fs-xs); color: var(--muted); }
  `,
})
export class ChatComposer {
  protected readonly store = inject(ChatStore);
  protected readonly status = inject(AiStatusStore);
  protected readonly images = inject(AiImageStore);
  private readonly browser = inject(ChatBrowserStore);
  private readonly launcher = inject(AiLauncher);
  private readonly platform = inject(Platform);
  private readonly workspace = inject(WorkspaceStore);
  private readonly dropTargets = inject(DropTargets);
  private readonly cardEl = viewChild<ElementRef<HTMLElement>>('card');
  private readonly inputEl = viewChild<ElementRef<HTMLTextAreaElement>>('input');

  /** The active chat (the browser menu is per chat). */
  readonly sessionId = input<string | null>(null);

  protected readonly draft = signal('');
  protected readonly dropping = signal(false);
  protected readonly contextModes = CONTEXT_MODES;
  protected readonly browserModes = BROWSER_MODES;
  protected readonly attachLabel = $localize`:@@ai.vision.chat.attach:Attach an image`;
  protected readonly sendTitle = $localize`:@@ai.chat.sendTitle:Send (Enter)`;
  protected readonly noSelection = $localize`:@@ai.chat.context.noSelection:Select events in the grid first`;

  protected readonly selectedCount = computed(() => this.store.sources().selectedIds.length);
  protected readonly browserStatus = computed(() => this.browser.statusOf(this.sessionId()));
  protected readonly browserMode = computed(() => chatBrowserMode(this.browserStatus()));

  protected readonly contextButton = computed(() => $localize`:@@ai.chat.context.button:Context: ${this.contextLabel(this.store.contextMode())}:mode:`);
  protected readonly browserButton = computed(() => $localize`:@@ai.chat.browser.button:Browser: ${this.browserLabel(this.browserMode())}:mode:`);
  /** What the current view sends (filter, time, instance, view), for the menu and the button's tooltip. */
  private readonly viewDetail = computed(() =>
    describeContext(this.store.sources(), (ms) => formatTime(ms, this.workspace.timezone()))
      .filter((c) => c.available && c.key !== 'selection')
      .map((c) => c.label)
      .join(' · '),
  );
  protected readonly contextTitle = computed(() => {
    const mode = this.store.contextMode();
    const detail = this.contextDetail(mode);
    return detail ? `${this.contextLabel(mode)}: ${detail}` : this.contextLabel(mode);
  });

  /** Why images cannot be attached now (null: they can). */
  protected readonly attachBlocked = computed<string | null>(() => {
    if (!this.status.hasModel()) return $localize`:@@ai.chat.blocked.setup:Set up the AI assistant first`;
    if (this.images.opening()) return $localize`:@@ai.vision.chat.opening:Opening an image…`;
    if (this.store.attachments().length >= MAX_IMAGES_PER_QUESTION) return $localize`:@@ai.vision.chat.full:At most ${MAX_IMAGES_PER_QUESTION}:max: images go with one question.`;
    return null;
  });
  protected readonly attachTitle = computed(() => this.attachBlocked() ?? $localize`:@@ai.vision.chat.attachTitle:Attach an image (or paste or drop one)`);

  /** Why Send is disabled (null: it is not). */
  protected readonly sendBlocked = computed<string | null>(() => {
    if (!this.status.hasModel()) return $localize`:@@ai.chat.blocked.setup:Set up the AI assistant first`;
    if (!this.draft().trim()) return $localize`:@@ai.chat.blocked.empty:Type a question`;
    return null;
  });

  protected readonly researchTitle = computed(() =>
    this.browserStatus()?.researchAuto
      ? $localize`:@@ai.chat.research.titleAuto:The AI searches the web, reads the top results (read-only) and answers with citations. It also does this by itself when a question needs it.`
      : $localize`:@@browser.chatBar.researchTitle:The AI searches the web, reads the top results (read-only) and answers with citations`,
  );

  /** Why the last attach failed (shown above the card with Retry). */
  protected readonly attachError = signal<string | null>(null);
  private lastAttach: (() => Promise<void>) | null = null;

  constructor() {
    // The card takes images dropped from the desktop (the shell routes OS drops by target, ADR 0025).
    effect((onCleanup) => {
      const el = this.cardEl()?.nativeElement;
      if (!el) return;
      onCleanup(this.dropTargets.register({ kind: 'aiImages', element: el, accept: (paths) => this.acceptDroppedPaths(paths) }));
    });
    // The question box follows its content (cleared after sending, too).
    effect(() => {
      this.draft();
      queueMicrotask(() => this.fit());
    });
  }

  protected contextLabel(mode: ContextMode): string {
    return contextModeLabel(mode, this.selectedCount());
  }

  protected contextDetail(mode: ContextMode): string | null {
    if (mode === 'none') return $localize`:@@ai.chat.context.noneDetail:Only your question`;
    const view = this.viewDetail() || $localize`:@@ai.chat.context.viewEmpty:No filter or time range`;
    return mode === 'selection' ? $localize`:@@ai.chat.context.selectionDetail:The selected events, and ${view}:view:` : view;
  }

  protected browserLabel(mode: ChatBrowserMode): string {
    switch (mode) {
      case 'off':
        return $localize`:Browser access off@@ai.chat.browser.off:Off`;
      case 'readAlong':
        return $localize`:@@ai.chat.browser.readAlong:Read along`;
      case 'agent':
        return $localize`:@@ai.chat.browser.agent:Agent`;
    }
  }

  protected browserHint(mode: ChatBrowserMode): string {
    switch (mode) {
      case 'off':
        return $localize`:@@ai.chat.browser.offHint:The AI does not use a browser in this chat`;
      case 'readAlong':
        return $localize`:@@ai.chat.browser.readAlongHint:When you ask about it, the AI may read the page open in the browser window it opens next to the app. It never browses by itself.`;
      case 'agent':
        return $localize`:@@ai.chat.browser.agentHint:A real browser window will open next to the app and the AI drives it step by step for this chat; it asks before new sites, typing and forms.`;
    }
  }

  protected setBrowser(mode: ChatBrowserMode): void {
    const id = this.sessionId();
    if (id) void this.browser.setMode(id, mode);
  }

  /** "Research…": the research dialog, prefilled with the draft (or, with none, the chat's last question). */
  protected research(): void {
    const question = this.draft().trim() || this.store.activeTurns().at(-1)?.user || undefined;
    void this.launcher.askResearch({ sessionId: this.sessionId(), question });
  }

  /**
   * Stop: in agent mode the agent is stopped (waiting steps are denied, agent mode ends and the answer says so),
   * otherwise the answer is cancelled.
   */
  protected stop(): void {
    const id = this.sessionId();
    if (id && this.browserStatus()?.agent) void this.browser.stop(id);
    else void this.store.cancel();
  }

  protected removeLabel(name: string): string {
    return $localize`:@@ai.vision.chat.remove:Remove ${name}:name:`;
  }

  protected onInput(e: Event): void {
    this.draft.set((e.target as HTMLTextAreaElement).value);
  }

  protected onEnter(e: Event): void {
    const k = e as KeyboardEvent;
    if (k.shiftKey || k.isComposing) return;
    k.preventDefault();
    if (!this.store.running()) void this.submit();
  }

  protected onEscape(e: Event): void {
    if (!this.store.running()) return;
    e.preventDefault();
    this.stop();
  }

  /** Grows the question box with its content, up to `COMPOSER_MAX_LINES` lines. */
  private fit(): void {
    const el = this.inputEl()?.nativeElement;
    if (!el) return;
    const css = getComputedStyle(el);
    const line = parseFloat(css.lineHeight) || 20;
    const padding = (parseFloat(css.paddingTop) || 0) + (parseFloat(css.paddingBottom) || 0);
    el.style.height = 'auto';
    el.style.height = `${Math.max(line + padding, composerHeight(el.scrollHeight, line, padding))}px`;
  }

  async submit(): Promise<void> {
    if (this.sendBlocked()) return;
    if (await this.store.send(this.draft())) this.draft.set('');
  }

  /** Puts the caret in the question box. */
  focus(): void {
    this.inputEl()?.nativeElement.focus();
  }

  /* ----------------------------------------------------------------- images */

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
   * Files dropped from the desktop onto the card (the OS drop, routed here by the shell as an `aiImages` target):
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
}
