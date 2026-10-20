import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';
import { AgentApprovalView, LinkView, PauseView, ResearchOfferView } from '../ai/chat/chat-model';
import { AgentDecision, Segment } from './browser.types';
import { handOffText, leakText } from './browser-text';

/** Text with the leak guard's findings highlighted (and named for screen readers). */
@Component({
  selector: 'imt-segments',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `@for (s of segments(); track $index) {@if (s.leak) {<mark [class]="s.leak" [title]="label(s)">{{ s.text }}<span class="sr-only"> ({{ label(s) }})</span></mark>} @else {{{ s.text }}}}`,
  styles: `
    :host { font-family: var(--font-mono); font-size: var(--fs-sm); overflow-wrap: anywhere; }
    mark { background: color-mix(in srgb, var(--warn-text) 20%, transparent); color: var(--text); border-radius: 3px; padding: 0 2px; outline: 1px solid var(--warn-text); }
    mark.private { background: transparent; outline-style: dashed; }
  `,
})
export class Segments {
  readonly segments = input.required<Segment[]>();

  protected label(s: Segment): string {
    return s.leak ? leakText(s.leak) : '';
  }
}

function askHeading(kind: AgentApprovalView['request']['ask']['kind'], site: string | null): string {
  const where = site ?? '';
  switch (kind) {
    case 'newSite':
      return $localize`:@@browser.approval.newSite:The AI wants to open ${where}:site:`;
    case 'leavingData':
      return $localize`:@@browser.approval.leavingData:The AI wants to send data from your logs to ${where}:site:`;
    case 'typeText':
      return $localize`:@@browser.approval.typeText:The AI wants to type on ${where}:site:`;
    case 'submitForm':
      return $localize`:@@browser.approval.submitForm:The AI wants to submit a form on ${where}:site:`;
    case 'pressButton':
      return $localize`:@@browser.approval.pressButton:The AI wants to press a button on ${where}:site:`;
  }
}

/** An agent step waiting for the user: what it does, exactly, with workspace data highlighted. */
@Component({
  selector: 'imt-agent-approval',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Segments],
  template: `
    <section class="card" role="group" [attr.aria-label]="heading()" data-testid="agent-approval">
      <p class="q"><strong>{{ heading() }}</strong></p>
      @if (ask().element; as el) {
        <p class="muted small">{{ el }}</p>
      }
      @if (a().request.urlSegments.length) {
        <p class="line"><span class="muted small" i18n="@@browser.approval.address">Address: </span><imt-segments [segments]="a().request.urlSegments" /></p>
      }
      @if (a().request.textSegments.length) {
        <p class="line"><span class="muted small" i18n="@@browser.approval.text">Text: </span><imt-segments [segments]="a().request.textSegments" /></p>
        @if (ask().submit) {
          <p class="muted small" i18n="@@browser.approval.thenSubmit">…and then press Enter.</p>
        }
      }
      @if (leaks()) {
        <p class="warn small" role="note" i18n="@@browser.approval.leakNote">Highlighted parts come from your logs or look private. Deny unless you want them sent.</p>
      }
      @if (a().state === 'pending' || a().state === 'sending') {
        <div class="row">
          <button type="button" class="btn ghost" [disabled]="a().state === 'sending'" (click)="decide.emit('deny')" i18n="@@browser.approval.deny">Deny</button>
          @if (ask().canAllowSite) {
            <button type="button" class="btn ghost" [disabled]="a().state === 'sending'" (click)="decide.emit('allowSite')" i18n="@@browser.approval.allowSite">Allow this site for this chat</button>
          }
          <button type="button" class="btn primary" [disabled]="a().state === 'sending'" (click)="decide.emit('allow')" i18n="@@browser.approval.allow">Allow</button>
        </div>
      } @else {
        <p class="result muted small" role="status">{{ outcome() }}</p>
      }
    </section>
  `,
  styles: `
    .card { border: 1px solid var(--border-3); border-radius: 6px; padding: 8px 10px; display: flex; flex-direction: column; gap: 4px; background: var(--bg-1); }
    p { margin: 0; }
    .small { font-size: var(--fs-sm); }
    .warn { color: var(--warn-text); }
    .row { display: flex; gap: 6px; justify-content: flex-end; flex-wrap: wrap; margin-top: 4px; }
  `,
})
export class AgentApprovalCard {
  readonly approval = input.required<AgentApprovalView>();
  readonly decide = output<AgentDecision>();
  protected readonly a = computed(() => this.approval());
  protected readonly ask = computed(() => this.approval().request.ask);
  protected readonly heading = computed(() => askHeading(this.ask().kind, this.ask().site));
  protected readonly leaks = computed(() => [...this.a().request.urlSegments, ...this.a().request.textSegments].some((s) => !!s.leak));
  protected readonly outcome = computed(() => {
    switch (this.a().state) {
      case 'allow':
        return $localize`:@@browser.approval.done.allowed:Allowed`;
      case 'allowSite':
        return $localize`:@@browser.approval.done.allowedSite:Allowed; this site is allowed for this chat`;
      case 'deny':
        return $localize`:@@browser.approval.done.denied:Denied`;
      case 'expired':
        return $localize`:@@browser.approval.done.expired:Not done (the answer ended first)`;
      default:
        return $localize`:@@browser.approval.done.sending:Sending your answer…`;
    }
  });
}

/** A search or page the AI suggests: the full address; nothing loads until the user opens it. */
@Component({
  selector: 'imt-link-card',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Segments],
  template: `
    <section class="card" role="group" [attr.aria-label]="link().title" data-testid="link-card">
      <p><strong>{{ link().title }}</strong> <span class="muted small">{{ kindText() }}</span></p>
      <p><imt-segments [segments]="segments()" /></p>
      <div class="row">
        <button type="button" class="btn ghost" (click)="open.emit(link().url)" i18n="@@browser.link.open">Open</button>
        <button type="button" class="btn" (click)="read.emit(link().url)" data-testid="link-card-read" title="Opens the page in IMT's browser, adds it to this chat as untrusted web content, and the AI goes on with it" i18n-title="@@browser.link.readTitle" i18n="@@browser.link.read">Open and let AI read it</button>
      </div>
    </section>
  `,
  styles: `
    .card { border: 1px solid var(--border-3); border-radius: 6px; padding: 8px 10px; display: flex; flex-direction: column; gap: 4px; }
    p { margin: 0; }
    .small { font-size: var(--fs-sm); }
    .row { display: flex; justify-content: flex-end; gap: 6px; flex-wrap: wrap; }
  `,
})
export class LinkCard {
  readonly link = input.required<LinkView>();
  readonly open = output<string>();
  readonly read = output<string>();
  protected readonly segments = computed(() => (this.link().segments.length ? this.link().segments : [{ text: this.link().url, leak: null }]));
  protected readonly kindText = computed(() =>
    this.link().kind === 'search' ? $localize`:@@browser.link.kindSearch:suggested search` : $localize`:@@browser.link.kindLink:suggested page`,
  );
}

/** The AI offers to research the web (automatic research is off, or the search text has workspace data in it). */
@Component({
  selector: 'imt-research-offer',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Segments],
  template: `
    <section class="card" role="group" [attr.aria-label]="heading()" data-testid="research-offer">
      <p><strong>{{ heading() }}</strong></p>
      <p><span class="muted small" i18n="@@browser.research.searchText">Search text: </span><imt-segments [segments]="segments()" /></p>
      @if (offer().needsConfirm) {
        <p class="warn small" role="note" i18n="@@browser.research.confirmNote">Highlighted parts come from your logs or look private: check the search text before it is sent.</p>
      }
      <div class="row">
        <button type="button" class="btn ghost" (click)="agent.emit()" data-testid="research-offer-agent" title="Turns on agent mode for this chat only: the AI drives the browser step by step and asks before new sites, typing and forms" i18n-title="@@browser.research.agentTitle" i18n="@@browser.research.agent">Let the AI drive the browser</button>
        <button type="button" class="btn primary" (click)="research.emit()" data-testid="research-offer-run" title="The AI searches and reads the top results by itself, read-only, and answers with citations" i18n-title="@@browser.research.runTitle" i18n="@@browser.research.run">Research this</button>
      </div>
    </section>
  `,
  styles: `
    .card { border: 1px solid var(--border-3); border-radius: 6px; padding: 8px 10px; display: flex; flex-direction: column; gap: 4px; }
    p { margin: 0; }
    .small { font-size: var(--fs-sm); }
    .warn { color: var(--warn-text); }
    .row { display: flex; justify-content: flex-end; gap: 6px; flex-wrap: wrap; }
  `,
})
export class ResearchOfferCard {
  readonly offer = input.required<ResearchOfferView>();
  readonly research = output<void>();
  readonly agent = output<void>();
  protected readonly heading = computed(() => $localize`:@@browser.research.heading:Research the web: ${this.offer().question}:question:`);
  protected readonly segments = computed(() => (this.offer().segments.length ? this.offer().segments : [{ text: this.offer().query, leak: null }]));
}

/** Why agent mode stopped, with Continue after the step cap. */
@Component({
  selector: 'imt-agent-paused',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="paused" role="status" data-testid="agent-paused">
      <p>{{ text() }}</p>
      @if (paused().reason === 'stepCap' && canContinue()) {
        <button type="button" class="btn" (click)="continue.emit()" data-testid="agent-continue" i18n="@@browser.paused.continue">Continue</button>
      }
    </div>
  `,
  styles: `
    .paused { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; padding: 6px 8px; border-left: 3px solid var(--accent); background: var(--bg-1); }
    p { margin: 0; font-size: var(--fs-sm); }
  `,
})
export class AgentPaused {
  readonly paused = input.required<PauseView>();
  readonly canContinue = input(true);
  readonly continue = output<void>();
  protected readonly text = computed(() => {
    const p = this.paused();
    switch (p.reason) {
      case 'stepCap':
        return $localize`:@@browser.paused.stepCap:Stopped after ${p.steps}:steps: steps. Continue?`;
      case 'stopped':
        return $localize`:@@browser.paused.stopped:You stopped the AI; the browser is yours.`;
      case 'handBack':
        return handOffText(p.handOff);
    }
  });
}
