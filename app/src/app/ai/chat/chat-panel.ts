import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, signal, untracked, viewChild } from '@angular/core';
import { formatTime } from '../../core/format';
import { Platform } from '../../core/platform';
import { WorkspaceStore } from '../../core/workspace.store';
import { OverwriteGuard } from '../../ui/overwrite-guard';
import { AiBackend, toAiError } from '../ai-backend';
import { AiChatPersistence } from '../ai-chat-persistence';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';
import { ApprovalDecision } from '../ai.types';
import { AiProposalCard } from '../proposals/proposal-card';
import { AiApprovalPrompt } from './approval-prompt';
import { describeContext, toolLabel, TurnView } from './chat-model';
import { ChatStore } from './chat.store';
import { AiCitations } from './citation-actions';
import { isNotFound } from './markdown';
import { AiRichText } from './rich-text';

/**
 * The AI assistant chat (a dock view). Answers stream in with the tools the model used, proposals to
 * review and apply, approval prompts for web searches, and evidence links into the grid and query bar.
 */
@Component({
  selector: 'imt-ai-chat-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AiRichText, AiProposalCard, AiApprovalPrompt],
  template: `
    <div class="bar" role="toolbar" aria-label="Chat">
      <label class="sr-only" for="ai-chat-session">Chat</label>
      <select id="ai-chat-session" class="select" [value]="store.activeId() ?? ''" (change)="store.select(asValue($event))" data-testid="ai-chat-sessions">
        @for (s of store.sessions(); track s.id) {
          <option [value]="s.id">{{ s.title || 'New chat' }}{{ s.running ? ' (answering…)' : '' }}</option>
        }
      </select>
      <button type="button" class="btn ghost" (click)="store.newChat()">New chat</button>
      <button type="button" class="btn ghost" [disabled]="!store.activeTurns().length" (click)="exportMarkdown()">Export…</button>
      <button type="button" class="btn ghost" [disabled]="!store.activeId()" aria-label="Delete this chat" title="Delete this chat" (click)="deleteChat()">Delete</button>
      <span class="spacer"></span>
      <span class="engine muted" data-testid="ai-engine-state">{{ engineLine() }}</span>
    </div>

    @if (status.needsSetup()) {
      <div class="setup" role="note">
        <p>The assistant needs a local model first. It runs fully on this computer; only the model file is downloaded.</p>
        <button type="button" class="btn primary" (click)="launcher.openSetup()">Set up AI assistant…</button>
      </div>
    }

    <div class="log" role="log" aria-live="polite" aria-label="Conversation" #log>
      @if (!store.activeTurns().length && !store.loading()) {
        <div class="empty">
          <p class="muted">Ask about the loaded logs. Answers cite events and queries you can open.</p>
          @if (store.prompts().length) {
            <ul class="starters" aria-label="Suggested questions">
              @for (p of store.prompts(); track p) {
                <li><button type="button" class="btn ghost starter" (click)="ask(p)">{{ p }}</button></li>
              }
            </ul>
          }
        </div>
      }
      @for (t of store.activeTurns(); track $index; let ti = $index) {
        <div class="turn">
          <p class="user"><span class="sr-only">You: </span>{{ t.user }}</p>
          <div class="answer" [class.neutral]="notFound(t)" [attr.data-testid]="'ai-answer-' + ti">
            @if (t.tools.length) {
              <ul class="tools" aria-label="Assistant activity">
                @for (c of t.tools; track $index) {
                  <li class="tool chip" [class.running]="c.running" [class.err]="c.error" data-testid="ai-tool-chip">
                    {{ label(c.name) }}@if (c.running) {<span class="dots" aria-hidden="true">…</span>}@if (c.summary) {<span class="sum">: {{ c.summary }}</span>}
                  </li>
                }
              </ul>
            }
            @for (a of t.approvals; track a.requestId) {
              <imt-ai-approval [approval]="a" (decide)="respond(a.requestId, $event)" />
            }
            @for (s of t.suggestions; track s.openUrl) {
              <p class="suggest">Suggested web search (nothing was sent): <code class="mono">{{ s.query }}</code>
                <button type="button" class="link" (click)="openWeb(s.openUrl, 'the search')">Search in my browser…</button></p>
            }
            @if (t.text) {
              <imt-ai-rich-text [text]="t.text" [citations]="t.citations" (record)="jump($event)" (query)="applyQuery($event)" (link)="openWeb($event.url, $event.title)" />
            } @else if (t.running && !t.approvals.length) {
              <p class="muted thinking" aria-label="Thinking">Thinking…</p>
            }
            @for (pv of t.proposals; track $index; let pi = $index) {
              <imt-ai-proposal-card [proposal]="pv.proposal" [state]="pv.state" (applied)="store.setProposalState(ti, pi, 'applied')" (dismissed)="store.setProposalState(ti, pi, 'dismissed')" />
            }
            @if (webSources(t).length) {
              <p class="sources muted">Web sources:
                @for (w of webSources(t); track w.url) {<button type="button" class="link" (click)="openWeb(w.url, w.title)">{{ w.title }}</button>}
              </p>
            }
            @for (n of t.notes; track $index) {
              <p class="note muted">{{ n }}</p>
            }
            @if (t.cancelled) {
              <p class="note muted">Stopped.</p>
            }
            @if (t.error) {
              <p class="err" role="alert">{{ t.error }} <button type="button" class="link" (click)="ask(t.user)">Ask again</button></p>
            }
          </div>
        </div>
      }
    </div>

    @if (store.error(); as e) {
      <p class="err banner" role="alert">{{ e }} <button type="button" class="link" (click)="store.dismissError()">Dismiss</button></p>
    }
    @if (notice(); as n) {
      <p class="ok banner" role="status">{{ n }}</p>
    }

    <div class="context">
      <button type="button" class="chip ctx" [attr.aria-expanded]="contextOpen()" aria-controls="ai-chat-context" (click)="contextOpen.set(!contextOpen())" data-testid="ai-context-chip">
        Context: {{ contextSummary() }}
      </button>
      @if (contextOpen()) {
        <fieldset id="ai-chat-context" class="ctx-list">
          <legend class="sr-only">What is sent with your question</legend>
          @for (c of contextLines(); track c.key) {
            <label class="ctx-item" [class.off]="!c.available">
              <input type="checkbox" [checked]="store.toggles()[c.key] && c.available" [disabled]="!c.available" (change)="store.toggle(c.key)" />
              {{ c.label }}
            </label>
          }
          <p class="muted small">The engine checks these and reads the events itself. Nothing leaves this computer.</p>
        </fieldset>
      }
    </div>

    <form class="composer" (submit)="$event.preventDefault(); submit()">
      <label class="sr-only" for="ai-chat-input">Ask about the logs</label>
      <textarea
        id="ai-chat-input"
        class="input"
        rows="2"
        maxlength="8000"
        placeholder="Ask about the logs…"
        [value]="draft()"
        [disabled]="!status.hasModel()"
        (input)="draft.set(asValue($event))"
        (keydown.enter)="onEnter($event)"
      ></textarea>
      @if (store.running()) {
        <button type="button" class="btn" (click)="store.cancel()">Stop</button>
      } @else {
        <button type="submit" class="btn primary" [disabled]="!draft().trim() || !status.hasModel()">Send</button>
      }
    </form>
  `,
  styles: `
    :host { display: flex; flex-direction: column; height: 100%; min-height: 0; font-size: 13px; }
    .bar { display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
    .bar .select { max-width: 220px; }
    .spacer { flex: 1; }
    .engine { font-size: 11px; }
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
    .tool { font-size: 11px; }
    .tool.running { opacity: 0.8; }
    .tool.err { border-color: var(--lvl-error); }
    .sum { color: var(--text-2); }
    .thinking { margin: 0; }
    .suggest, .sources, .note { margin: 0; font-size: 12px; }
    .sources .link { margin-left: 6px; }
    .err { color: var(--danger-text, var(--lvl-error)); margin: 0; }
    .ok { color: var(--ok); margin: 0; }
    .banner { padding: 4px 12px; font-size: 12px; }
    .link { all: unset; cursor: pointer; color: var(--accent); text-decoration: underline; }
    .link:focus-visible { outline: 2px solid var(--accent); }
    .context { padding: 4px 8px 0; }
    .ctx { cursor: pointer; font-size: 11px; max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .ctx-list { border: 1px solid var(--border); border-radius: 6px; margin: 4px 0 0; padding: 6px 8px; display: flex; flex-direction: column; gap: 2px; font-size: 12px; }
    .ctx-item.off { color: var(--muted); }
    .small { margin: 4px 0 0; font-size: 11px; }
    .composer { display: flex; gap: 6px; padding: 8px; align-items: flex-end; }
    .composer textarea { flex: 1; resize: vertical; min-height: 38px; font-family: inherit; }
  `,
})
export class AiChatPanel {
  protected readonly store = inject(ChatStore);
  protected readonly status = inject(AiStatusStore);
  protected readonly launcher = inject(AiLauncher);
  private readonly citations = inject(AiCitations);
  private readonly backend = inject(AiBackend);
  private readonly platform = inject(Platform);
  private readonly overwrite = inject(OverwriteGuard);
  private readonly persistence = inject(AiChatPersistence);
  private readonly workspace = inject(WorkspaceStore);
  private readonly logEl = viewChild<ElementRef<HTMLElement>>('log');

  protected readonly draft = signal('');
  protected readonly contextOpen = signal(false);
  protected readonly notice = signal<string | null>(null);

  protected readonly contextLines = computed(() => describeContext(this.store.sources(), (ms) => formatTime(ms, this.workspace.timezone())));
  protected readonly contextSummary = computed(() => {
    const on = this.store.toggles();
    const parts = this.contextLines()
      .filter((c) => c.available && on[c.key])
      .map((c) => c.label.split(':')[0].toLowerCase());
    return parts.length ? parts.join(', ') : 'nothing selected';
  });
  protected readonly engineLine = computed(() => {
    const s = this.status.status();
    if (!s) return '';
    if (!s.available) return 'AI unavailable';
    const model = s.model ? s.model.replace(/\.gguf$/i, '') : 'no model';
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

  protected label(name: string): string {
    return toolLabel(name);
  }

  protected notFound(t: TurnView): boolean {
    return !t.running && isNotFound(t.text);
  }

  protected webSources(t: TurnView): { url: string; title: string }[] {
    return t.running ? [] : t.citations.flatMap((c) => (c.kind === 'web' ? [{ url: c.url, title: c.title }] : []));
  }

  protected onEnter(e: Event): void {
    const k = e as KeyboardEvent;
    if (k.shiftKey || k.isComposing) return;
    k.preventDefault();
    void this.submit();
  }

  async submit(): Promise<void> {
    if (await this.store.send(this.draft())) this.draft.set('');
  }

  async ask(question: string): Promise<void> {
    await this.store.send(question);
  }

  protected respond(requestId: number, d: ApprovalDecision): void {
    void this.store.respond(requestId, d);
  }

  protected async jump(id: number): Promise<void> {
    await this.citations.record(id);
  }

  protected async applyQuery(ql: string): Promise<void> {
    const err = await this.citations.query(ql);
    this.notice.set(err ? `Not applied: ${err}` : `Applied ${ql}`);
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
    const path = await this.platform.pickSavePath('Export chat', `${name}.md`, [{ name: 'Markdown', extensions: ['md'] }]);
    if (!path) return;
    try {
      const written = await this.overwrite.write(path, (o) => this.backend.chatExportMarkdown(id, path, o));
      if (written !== null) this.notice.set(`Exported to ${written}`);
    } catch (e) {
      this.notice.set(`Not exported: ${toAiError(e).message}`);
    }
  }
}
