import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { AiStatusStore } from '../../ai/ai-status.store';
import { IssueMatch } from '../../core/api.types';
import { formatCount } from '../../core/format';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { Icon } from '../../shared/icon';
import { KnownIssuesStore } from './known-issues.store';

/** The knowledge-file block that declares a known issue (shown in the empty state). */
export const KNOWN_ISSUE_EXAMPLE = '```known-issue\ntitle: Payment gateway timeouts\npattern: (?i)gateway.*timed out\nfix: Raise the gateway timeout to 30 s.\n```';

/**
 * The "Known issues" section of the Problems pane (ADR 0039): the issues declared in the knowledge files that are
 * present in the open data, with their fix and evidence, and the AI's suggestions, which the engine verified. All
 * texts come from the engine and are shown as plain text.
 */
@Component({
  selector: 'imt-known-issues',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, NgTemplateOutlet],
  template: `
    <section class="issues" aria-labelledby="known-issues-title" data-testid="known-issues">
      <div class="head">
        <button type="button" class="icon-btn" data-testid="known-issues-toggle" [attr.aria-expanded]="expanded()" aria-controls="known-issues-body" aria-label="Show or hide the known issues" i18n-aria-label="@@known.issues.toggle" (click)="toggle()">
          <imt-icon [name]="expanded() ? 'chevronDown' : 'chevronRight'" [size]="12" />
        </button>
        <h3 id="known-issues-title" i18n="@@known.issues.title">Known issues ({{ total() }})</h3>
        <span class="spacer"></span>
        <button
          type="button"
          class="btn"
          data-testid="known-issues-suggest"
          [disabled]="!ai.hasModel() || store.suggesting()"
          [attr.title]="ai.hasModel() ? null : noModelHint"
          (click)="store.suggest()"
        >
          <imt-icon name="ai" [size]="12" />
          <span>{{ store.suggesting() ? suggestingLabel : suggestLabel }}</span>
        </button>
        <button type="button" class="icon-btn" data-testid="known-issues-refresh" aria-label="Check known issues again" i18n-aria-label="@@known.issues.refresh" title="Check again" i18n-title="@@known.issues.refreshTitle" [disabled]="store.loading()" (click)="store.load()">
          <imt-icon name="refresh" [size]="13" />
        </button>
      </div>
      <div id="known-issues-body" class="body" [hidden]="!expanded()">
      @if (!ai.hasModel()) {
        <div class="hint faint" data-testid="known-issues-no-model">{{ noModelHint }}</div>
      }
      @if (store.error(); as error) {
        <div class="error" role="alert">
          <span i18n="@@known.issues.loadError">Known issues could not be checked: {{ error }}</span>
          <button type="button" class="btn" (click)="store.load()" i18n="@@known.issues.retry">Retry</button>
        </div>
      }
      @if (store.suggestError(); as error) {
        <div class="error" role="alert" data-testid="known-issues-suggest-error">
          <span>{{ error }}</span>
          <button type="button" class="icon-btn" aria-label="Dismiss" i18n-aria-label="@@known.issues.dismiss" (click)="store.clearSuggestions()"><imt-icon name="close" [size]="12" /></button>
        </div>
      }
      @if (store.suggesting()) {
        <div class="hint" role="status" data-testid="known-issues-progress" i18n="@@known.issues.suggesting">The AI is reading the knowledge notes; the engine checks each suggestion on the open data…</div>
      }
      @if (store.report()?.matches; as matches) {
        <ul class="list" aria-label="Known issues found" i18n-aria-label="@@known.issues.list">
          @for (m of matches; track m.issue.id) {
            <ng-container *ngTemplateOutlet="card; context: { $implicit: m }" />
          }
        </ul>
      }
      @if (store.suggestions().length) {
        <h4 class="sub" i18n="@@known.issues.suggestions">Suggested by the AI</h4>
        <ul class="list" aria-label="Suggested known issues" i18n-aria-label="@@known.issues.suggestionList">
          @for (m of store.suggestions(); track m.issue.id) {
            <ng-container *ngTemplateOutlet="card; context: { $implicit: m }" />
          }
        </ul>
      }
      @if (empty()) {
        <div class="empty" data-testid="known-issues-empty">
          <p i18n="@@known.issues.empty">No known issue matches the open data. Declare one in a knowledge file (Settings > AI > Knowledge) with a fenced block:</p>
          <pre class="example">{{ example }}</pre>
          <p class="faint" i18n="@@known.issues.emptyHint">Use pattern: for a regex over the message, or query: for a query-language filter. fix: is what to do about it.</p>
        </div>
      }
      @if (store.report()?.warnings; as warnings) {
        @if (warnings.length) {
          <details class="warnings" data-testid="known-issues-warnings">
            <summary i18n="@@known.issues.warnings">{{ warnings.length }} knowledge notes need fixing</summary>
            <ul>
              @for (w of warnings; track $index) {
                <li>{{ w }}</li>
              }
            </ul>
          </details>
        }
      }
      </div>
    </section>

    <ng-template #card let-m>
      <li class="card" [attr.data-testid]="'known-issue-' + m.issue.id" (click)="cardClick($event, m)">
        <div class="line">
          <strong class="name">{{ m.issue.title }}</strong>
          <span class="badge" [class.ai]="m.origin === 'ai'" data-testid="known-issue-origin">{{ originLabel(m) }}</span>
          <span class="count" data-testid="known-issue-count">{{ countLabel(m) }}</span>
        </div>
        <div class="fix" data-testid="known-issue-fix">{{ m.issue.fix }}</div>
        <div class="line meta">
          <span class="faint" i18n="@@known.issues.file">From {{ m.issue.file }}</span>
          <span class="spacer"></span>
          <span class="row-actions">
          @if (m.origin === 'ai') {
            <button type="button" class="btn" data-testid="known-issue-accept" [attr.aria-pressed]="isAccepted(m)" (click)="store.toggleAccepted(m.issue.id)">{{ isAccepted(m) ? acceptedLabel : acceptLabel }}</button>
          }
          <button type="button" class="btn ghost" data-testid="known-issue-show" (click)="show(m)" i18n="@@known.issues.show">Show events</button>
          <button type="button" class="btn ghost" data-testid="known-issue-select" (click)="selectEvidence(m)" i18n="@@known.issues.selectEvidence">Select evidence</button>
          </span>
        </div>
      </li>
    </ng-template>
  `,
  styles: `
    :host { display: block; flex: none; border-bottom: 1px solid var(--border); font-size: var(--fs-sm); color: var(--text-2); }
    .issues { padding: 6px 12px; }
    .body[hidden] { display: none; }
    .head { display: flex; align-items: center; gap: 8px; }
    h3 { margin: 0; font-size: var(--fs-md); color: var(--text); }
    .sub { margin: 8px 0 4px; font-size: var(--fs-sm); color: var(--text); }
    .spacer { flex: 1; }
    .hint { padding: 4px 0; }
    .error { display: flex; align-items: center; gap: 8px; padding: 4px 0; color: var(--danger-text); }
    .error span { flex: 1; }
    .list { list-style: none; margin: 6px 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    .card { border: 1px solid var(--border); border-radius: 6px; padding: 6px 8px; background: var(--bg-2); }
    .line { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .name { color: var(--text); }
    .badge { font-size: var(--fs-xs); color: var(--muted); }
    .badge.ai { color: var(--accent-text); }
    .count { color: var(--muted); font-size: var(--fs-xs); }
    .fix { white-space: pre-wrap; margin: 4px 0; color: var(--text); }
    .meta { font-size: var(--fs-xs); }
    .card { cursor: pointer; }
    /* A card's actions show on hover and keyboard focus; clicking the card shows its events. */
    .row-actions { display: flex; gap: 4px; opacity: 0; }
    .card:hover .row-actions, .card:focus-within .row-actions { opacity: 1; }
    @media (hover: none) { .row-actions { opacity: 1; } }
    .example { margin: 4px 0; padding: 6px 8px; background: var(--bg-2); border: 1px solid var(--border); border-radius: 4px; overflow: auto; user-select: text; }
    .warnings { margin: 6px 0; }
    .warnings summary { cursor: pointer; }
    .warnings li { white-space: pre-wrap; }
    p { margin: 4px 0; }
  `,
})
export class KnownIssues {
  protected readonly store = inject(KnownIssuesStore);
  protected readonly ai = inject(AiStatusStore);
  private readonly ctx = inject(COMMAND_CONTEXT);

  protected readonly example = KNOWN_ISSUE_EXAMPLE;
  protected readonly suggestLabel = $localize`:@@known.issues.suggest:Suggest with AI`;
  protected readonly suggestingLabel = $localize`:@@known.issues.suggestingLabel:Suggesting…`;
  protected readonly acceptLabel = $localize`:@@known.issues.accept:Accept for report`;
  protected readonly acceptedLabel = $localize`:@@known.issues.accepted:Accepted`;
  protected readonly noModelHint = $localize`:@@known.issues.needsModel:Needs an AI model: set one up in Settings > AI`;

  protected readonly total = computed(() => (this.store.report()?.matches.length ?? 0) + this.store.suggestions().length);
  protected readonly empty = computed(() => this.store.report() !== null && this.total() === 0 && !this.store.error());

  /** The user's choice; until they make one the section is open exactly when it has something to show. */
  private readonly userExpanded = signal<boolean | null>(null);
  protected readonly expanded = computed(() => this.userExpanded() ?? this.total() > 0);

  constructor() {
    // Matched while the Problems pane shows this section, and again when the workspace changes.
    this.store.gate.use();
  }

  protected originLabel(m: IssueMatch): string {
    return m.origin === 'ai' ? $localize`:@@known.issues.origin.ai:AI suggested, verified by the engine` : $localize`:@@known.issues.origin.rule:From rule`;
  }

  protected countLabel(m: IssueMatch): string {
    const n = formatCount(m.count);
    return $localize`:@@known.issues.count:${n}:n: events`;
  }

  protected isAccepted(m: IssueMatch): boolean {
    return this.store.accepted().includes(m.issue.id);
  }

  protected toggle(): void {
    this.userExpanded.set(!this.expanded());
  }

  /** A click on the card (not on one of its buttons) shows its events. */
  protected cardClick(e: Event, m: IssueMatch): void {
    if (!(e.target as HTMLElement).closest('button')) this.show(m);
  }

  /** Filters the events to the match's query and selects its first sample event (the evidence) in the grid. */
  protected selectEvidence(m: IssueMatch): void {
    this.show(m);
    const first = m.sampleIds[0];
    if (first !== undefined) void this.ctx.jumpTo(first);
  }

  /** Filters the workspace to the events of the match and shows them in the grid. */
  protected show(m: IssueMatch): void {
    this.ctx.setQuery({ ql: m.query });
    this.ctx.showView('events');
  }
}
