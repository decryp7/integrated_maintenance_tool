import { formatCount } from '../../shared/format/units';
import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { DIALOG_REF } from '../../ui/dialog.service';
import { AiLauncher } from '../ai-launcher';
import { AiStatusStore } from '../ai-status.store';
import { FolderSetupStore } from './folder-setup.store';
import { ProposalGroup, SetupProposal, SurveyStage } from './folder-setup.types';
import { FolderSetupPreview } from './proposal-preview';

export const GROUP_TITLES: Record<ProposalGroup, string> = {
  formats: $localize`:@@ai.folderSetup.group.formats:Formats`,
  instances: $localize`:@@ai.folderSetup.group.instances:Instances and lanes`,
  markers: $localize`:@@ai.folderSetup.group.markers:Markers and runs`,
  extractors: $localize`:@@ai.folderSetup.group.extractors:Extractors`,
  noise: $localize`:@@ai.folderSetup.group.noise:Noise reduction`,
  charts: $localize`:@@ai.folderSetup.group.charts:Charts and dashboard`,
  queries: $localize`:@@ai.folderSetup.group.queries:Saved queries`,
};

const STAGE_LABELS: Record<SurveyStage, string> = {
  sampling: $localize`:@@ai.folderSetup.stage.sampling:Sampling the files…`,
  analysing: $localize`:@@ai.folderSetup.stage.analysing:Looking at fields, messages and numbers…`,
  proposing: $localize`:@@ai.folderSetup.stage.proposing:Writing proposals…`,
  validating: $localize`:@@ai.folderSetup.stage.validating:Checking every proposal on the sample…`,
  done: $localize`:@@ai.folderSetup.stage.done:Done`,
};

type Step = 'survey' | 'proposals' | 'review' | 'apply';

/**
 * "Set up analysis for this folder…": Survey (progress, cancel) → Proposals grouped with their previews →
 * Review of the combined profile → Apply, then links to the views that changed. Every number shown is the
 * engine's; nothing is applied before Apply.
 */
@Component({
  selector: 'imt-folder-setup-wizard',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FolderSetupPreview],
  template: `
    <ol class="steps" aria-label="Steps" i18n-aria-label="@@ai.folderSetup.steps">
      @for (s of steps; track s.id) {
        <li [class.current]="step() === s.id" [attr.aria-current]="step() === s.id ? 'step' : null">{{ s.label }}</li>
      }
    </ol>

    @switch (store.phase()) {
      @case ('surveying') {
        <section class="pane" aria-live="polite">
          <p>{{ stageLabel() }}</p>
          <div class="progress" role="progressbar" aria-label="Survey progress" i18n-aria-label="@@ai.folderSetup.progressLabel" aria-valuemin="0" [attr.aria-valuemax]="store.progress()?.total ?? 1" [attr.aria-valuenow]="store.progress()?.done ?? 0">
            <span class="fill" [style.width.%]="percent()"></span>
          </div>
          <div class="actions">
            <button type="button" class="btn" data-testid="survey-cancel" (click)="store.cancel()" i18n="@@ai.folderSetup.cancelSurvey">Cancel survey</button>
          </div>
        </section>
      }
      @case ('failed') {
        <section class="pane">
          <p class="err" role="alert">{{ store.error() }}</p>
          <div class="actions"><button type="button" class="btn primary" (click)="store.start()" i18n="@@ai.folderSetup.retry">Try again</button></div>
        </section>
      }
      @case ('cancelled') {
        <section class="pane">
          <p role="status" i18n="@@ai.folderSetup.cancelled">The survey was cancelled. Nothing was changed.</p>
          <div class="actions"><button type="button" class="btn primary" (click)="store.start()" i18n="@@ai.folderSetup.surveyAgain">Survey again</button></div>
        </section>
      }
      @case ('proposals') {
        <section class="pane">
          @if (store.survey(); as s) {
            <p class="facts" data-testid="survey-facts"><ng-container i18n="@@ai.folderSetup.facts">Sampled {{ fmt(s.sampledEvents) }} of {{ fmt(s.totalEvents) }} events in {s.groups.length, plural, =1 {1 group of files} other {{{ s.groups.length }} groups of files}}.</ng-container>@if (s.unrecognised.length) {<ng-container i18n="@@ai.folderSetup.factsUnread"> {s.unrecognised.length, plural, =1 {1 kind of file is} other {{{ s.unrecognised.length }} kinds of files are}} not read.</ng-container>}</p>
            @if (s.truncated) {
              <p class="facts" i18n="@@ai.folderSetup.truncated">The folder is large: the proposals are based on a sample.</p>
            }
          }
          <div class="ai" data-testid="ai-block">
            @if (ai.hasModel()) {
              @switch (store.ai()) {
                @case ('running') {
                  <span role="status" i18n="@@ai.folderSetup.ai.running">The AI is refining the proposals…</span>
                  <button type="button" class="btn ghost" (click)="store.cancelAi()" i18n="@@ai.folderSetup.ai.cancel">Stop</button>
                }
                @case ('done') {
                  <span role="status" i18n="@@ai.folderSetup.ai.done">AI proposals added below (marked AI, checked by the engine, not selected).</span>
                }
                @default {
                  <button type="button" class="btn" data-testid="ai-refine" (click)="store.refineWithAi()" i18n="@@ai.folderSetup.ai.refine">Improve with AI</button>
                  <span class="muted" i18n="@@ai.folderSetup.ai.hint">Better names, regexes and formats, grounded in this survey.</span>
                }
              }
              @if (store.aiMessage(); as m) {
                <p class="err" role="alert">{{ m }}</p>
              }
              @for (n of store.aiNotes(); track $index) {
                <p class="facts">{{ n }}</p>
              }
            } @else {
              <span class="muted" data-testid="ai-off-note" i18n="@@ai.folderSetup.ai.off">These proposals come from the engine's heuristics. Set up the local AI assistant for better names, regexes and formats.</span>
              @if (ai.available()) {
                <button type="button" class="btn ghost" (click)="setupAi()" i18n="@@ai.folderSetup.ai.setup">Set up AI…</button>
              }
            }
          </div>

          @if (!store.proposals().length) {
            <p role="status" i18n="@@ai.folderSetup.none">Nothing to propose: the current profile already fits these files.</p>
          }
          @for (g of store.groups(); track g.group) {
            <fieldset class="group" [attr.data-group]="g.group">
              <legend>{{ groupTitle(g.group) }}</legend>
              @for (p of g.proposals; track p.id) {
                <div class="proposal" [attr.data-testid]="'proposal-' + p.id">
                  <label class="head">
                    <input type="checkbox" [checked]="store.isSelected(p.id)" (change)="store.toggle(p.id)" />
                    <span class="title">{{ p.title }}</span>
                    @if (p.origin === 'ai') {
                      <span class="badge" i18n="@@ai.folderSetup.badge.ai">AI</span>
                    }
                  </label>
                  <p class="summary">{{ p.summary }}</p>
                  @if (p.requires.length) {
                    <p class="facts" i18n="@@ai.folderSetup.requires">Needs: {{ requiresText(p) }}</p>
                  }
                  <imt-folder-setup-preview [preview]="p.preview" />
                  <details><summary i18n="@@ai.folderSetup.asToml">As profile TOML</summary><pre class="mono">{{ p.toml }}</pre></details>
                </div>
              }
            </fieldset>
          }
          @if (store.skipped().length) {
            <details class="skipped" data-testid="skipped">
              <summary i18n="@@ai.folderSetup.skipped">Skipped proposals ({{ store.skipped().length }})</summary>
              <ul>
                @for (s of store.skipped(); track s.id) {
                  <li><span class="title">{{ s.title }}</span>@if (s.origin === 'ai') { <span class="badge" i18n="@@ai.folderSetup.badge.ai">AI</span>}: <span class="muted">{{ s.reason }}</span></li>
                }
              </ul>
            </details>
          }
          @if (store.error(); as e) {
            <p class="err" role="alert">{{ e }}</p>
          }
          <div class="actions sticky">
            <button type="button" class="btn ghost" (click)="close()" i18n="@@ai.folderSetup.close">Close</button>
            <button type="button" class="btn primary" data-testid="review" [disabled]="!store.canReview()" (click)="store.review()" i18n="@@ai.folderSetup.review">Review {store.selected().length, plural, =1 {1 change} other {{{ store.selected().length }} changes}}…</button>
          </div>
        </section>
      }
      @case ('review') {
        <section class="pane">
          @if (store.composed(); as c) {
            <p class="facts" data-testid="diff-summary"><ng-container i18n="@@ai.folderSetup.diffSummary">The profile changes by {{ c.added }} added and {{ c.removed }} removed lines.</ng-container>@if (c.savedQueries.length) {<ng-container i18n="@@ai.folderSetup.diffQueries"> {c.savedQueries.length, plural, =1 {1 saved query is} other {{{ c.savedQueries.length }} saved queries are}} added to the project.</ng-container>}</p>
            <pre class="diff mono" data-testid="diff" tabindex="0" aria-label="Profile changes" i18n-aria-label="@@ai.folderSetup.diffLabel">@for (l of diffLines(); track $index) {<span [class]="l.op">{{ prefix(l.op) }}{{ l.text }}
</span>}</pre>
          }
          @if (store.error(); as e) {
            <p class="err" role="alert">{{ e }}</p>
          }
          <div class="actions">
            <button type="button" class="btn ghost" (click)="store.back()" i18n="@@ai.folderSetup.back">Back</button>
            <button type="button" class="btn primary" data-testid="apply" (click)="store.apply()" i18n="@@ai.folderSetup.apply">Apply to the profile</button>
          </div>
        </section>
      }
      @case ('applying') {
        <section class="pane"><p role="status" i18n="@@ai.folderSetup.applying">Saving the profile and reading the files again…</p></section>
      }
      @case ('applied') {
        <section class="pane">
          <p class="ok" role="status" data-testid="applied" i18n="@@ai.folderSetup.applied">Applied. Edit ▸ Undo takes the whole set-up back.</p>
          <div class="actions start">
            <button type="button" class="btn primary" data-testid="open-timeline" (click)="openView('timeline')" i18n="@@ai.folderSetup.openTimeline">Open the timeline</button>
            @if (changed('chart') || changed('dashboard')) {
              <button type="button" class="btn" (click)="openView(changed('dashboard') ? 'dashboards' : 'metrics')" i18n="@@ai.folderSetup.openCharts">Open charts</button>
            }
            @if (changed('savedQuery')) {
              <button type="button" class="btn" (click)="openView('bookmarks')" i18n="@@ai.folderSetup.openQueries">Open saved queries</button>
            }
            <button type="button" class="btn" (click)="openView('profile')" i18n="@@ai.folderSetup.openProfile">Open the profile</button>
            <button type="button" class="btn ghost" (click)="close()" i18n="@@ai.folderSetup.done">Done</button>
          </div>
        </section>
      }
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; max-height: min(78vh, 900px); overflow: auto; }
    .steps { display: flex; gap: 6px; list-style: none; margin: 0; padding: 0; font-size: var(--fs-sm); color: var(--muted); }
    .steps li { padding: 2px 8px; border-radius: 10px; border: 1px solid var(--border); }
    .steps li.current { color: var(--text); border-color: var(--accent); font-weight: 600; }
    .pane { display: flex; flex-direction: column; gap: 10px; }
    .progress { height: 8px; background: var(--bg-2); border-radius: 4px; overflow: hidden; }
    .progress .fill { display: block; height: 100%; background: var(--accent); transition: width 0.15s; }
    .facts, .summary { margin: 0; font-size: var(--fs-sm); color: var(--text-2); }
    .ai { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 8px 10px; border: 1px dashed var(--border-2); border-radius: 6px; font-size: var(--fs-sm); }
    .ai p { flex-basis: 100%; }
    .group { border: 1px solid var(--border); border-radius: 8px; padding: 6px 10px 10px; margin: 0; display: flex; flex-direction: column; gap: 10px; }
    legend { font-weight: 600; padding: 0 4px; }
    .proposal { display: flex; flex-direction: column; gap: 4px; }
    .head { display: flex; align-items: center; gap: 8px; cursor: pointer; }
    .title { font-weight: 500; overflow-wrap: anywhere; }
    details summary { cursor: pointer; font-size: var(--fs-sm); color: var(--muted); }
    pre { max-height: 220px; overflow: auto; background: var(--code-bg); padding: 6px 8px; border-radius: 4px; font-size: var(--fs-xs); margin: 4px 0 0; }
    .diff { max-height: 50vh; }
    .diff .add { color: var(--ok); }
    .diff .remove { color: var(--danger-text); }
    .diff .same { color: var(--muted); }
    .skipped ul { margin: 4px 0 0; padding-left: 18px; font-size: var(--fs-sm); }
    .err { margin: 0; color: var(--danger-text); font-size: var(--fs-sm); }
    .ok { margin: 0; color: var(--ok); }
    .actions { display: flex; justify-content: flex-end; gap: 8px; flex-wrap: wrap; }
    .actions.start { justify-content: flex-start; }
    .actions.sticky { position: sticky; bottom: 0; background: var(--bg-1); padding-top: 6px; }
  `,
})
export class FolderSetupWizard {
  protected readonly fmt = formatCount;
  protected readonly store = inject(FolderSetupStore);
  protected readonly ai = inject(AiStatusStore);
  private readonly launcher = inject(AiLauncher);
  private readonly ctx = inject(COMMAND_CONTEXT);
  private readonly ref = inject(DIALOG_REF);

  protected readonly steps: { id: Step; label: string }[] = [
    { id: 'survey', label: $localize`:Wizard step@@ai.folderSetup.step.survey:Survey` },
    { id: 'proposals', label: $localize`:Wizard step@@ai.folderSetup.step.proposals:Proposals` },
    { id: 'review', label: $localize`:Wizard step@@ai.folderSetup.step.review:Review` },
    { id: 'apply', label: $localize`:Wizard step@@ai.folderSetup.step.apply:Apply` },
  ];

  protected readonly step = computed<Step>(() => {
    switch (this.store.phase()) {
      case 'proposals':
        return 'proposals';
      case 'review':
        return 'review';
      case 'applying':
      case 'applied':
        return 'apply';
      default:
        return 'survey';
    }
  });

  protected readonly stageLabel = computed(() => STAGE_LABELS[this.store.progress()?.stage ?? 'sampling']);
  protected readonly percent = computed(() => {
    const p = this.store.progress();
    return p && p.total ? Math.round((p.done / p.total) * 100) : 0;
  });
  /** The diff around the changes (unchanged runs longer than 6 lines are cut to 3 lines of context). */
  protected readonly diffLines = computed(() => {
    const diff = this.store.composed()?.diff ?? [];
    const near = (i: number) => diff.slice(Math.max(0, i - 3), i + 4).some((l) => l.op !== 'same');
    return diff.filter((l, i) => l.op !== 'same' || near(i));
  });

  protected groupTitle(g: ProposalGroup): string {
    return GROUP_TITLES[g];
  }

  protected requiresText(p: SetupProposal): string {
    return p.requires.map((id) => this.store.proposals().find((x) => x.id === id)?.title ?? id).join('; ');
  }

  protected prefix(op: string): string {
    return op === 'add' ? '+ ' : op === 'remove' ? '- ' : '  ';
  }

  /** Whether the applied selection changed this kind of thing. */
  protected changed(kind: string): boolean {
    return this.store.selectedProposals().some((p) => p.fragments.some((f) => f.kind === kind));
  }

  protected openView(id: string): void {
    this.ctx.showView(id);
    this.close();
  }

  protected setupAi(): void {
    void this.launcher.openSetup();
  }

  protected close(): void {
    this.ref.close();
  }
}
