import { ChangeDetectionStrategy, Component, computed, effect, inject, input, OnInit, signal } from '@angular/core';
import { DIALOG_REF } from '../../ui/dialog.service';
import { AiBackend, toAiError } from '../ai-backend';
import { fitAdvice, fitFacts, fitLabel, fitNumbers, formatBytes, hardwareLines, speedLabel, visionLabel } from '../ai-format';
import { AiStatusStore } from '../ai-status.store';
import { groupCode } from '../pitwall-format';
import { Recommendation } from '../ai.types';
import { AiRuntimeStore } from '../runtimes/ai-runtime.store';
import { canStart, choices, INITIAL, next, selected, WizardEvent, WizardState } from './wizard';

/**
 * "Set up AI assistant": a modal choice. It shows this computer's hardware and the models that fit it; "Download and
 * set up" closes the dialog at once and the engine does the rest as a background job (the inference runtime when none
 * is installed, then the model: download, check, load, test), shown in Jobs, the status bar and the assistant pane,
 * with a message when it is ready or has failed. Opened in a dialog.
 */
@Component({
  selector: 'debrief-ai-setup-wizard',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (pitwallChoice()) {
      <section class="pitwall" aria-labelledby="ai-pitwall-title" data-testid="ai-setup-pitwall">
        @if (usingPitwall(); as used) {
          <h3 id="ai-pitwall-title" i18n="@@ai.setup.pitwall.inUseTitle">Pitwall is your AI engine</h3>
          <p data-testid="ai-setup-pitwall-inuse" i18n="@@ai.setup.pitwall.inUse">Debrief uses Pitwall (model {{ used.model ?? '' }}).</p>
        } @else {
          <h3 id="ai-pitwall-title" i18n="@@ai.setup.pitwall.title">Pitwall is running on this computer</h3>
          <p i18n="@@ai.setup.pitwall.text">Debrief can use Pitwall's models instead of downloading its own copy. Everything stays on this computer.</p>
        }
        @if (pitwall()?.pairing; as pairing) {
          <p class="code" data-testid="ai-setup-pitwall-code" aria-label="Pairing code" i18n-aria-label="@@ai.settings.pitwall.codeLabel">{{ group(pairing.code) }}</p>
          <p class="muted" i18n="@@ai.settings.pitwall.approve">Approve Debrief in Pitwall ▸ Apps & keys and check it shows this code.</p>
        }
        @if (pairingError(); as e) {
          <p class="err" role="alert" data-testid="ai-setup-pitwall-error">{{ e }}</p>
        }
        <div class="inline-actions">
          @if (usingPitwall()) {
            <button type="button" class="btn primary" data-testid="ai-setup-pitwall-close" (click)="close()" i18n="@@ai.setup.close">Close</button>
          } @else if (pitwall()?.pairing) {
            <button type="button" class="btn" data-testid="ai-setup-pitwall-cancel" (click)="cancelPitwall()" i18n="@@ai.setup.pitwall.cancel">Cancel</button>
          } @else {
            <button type="button" class="btn primary" data-testid="ai-setup-use-pitwall" [disabled]="connecting()" (click)="usePitwall()" i18n="@@ai.setup.pitwall.use">Use Pitwall</button>
          }
          <button type="button" class="btn" data-testid="ai-setup-download-instead" (click)="downloadInstead.set(true)" i18n="@@ai.setup.pitwall.downloadInstead">Download a model for Debrief instead</button>
        </div>
      </section>
    } @else {
    <p class="local" data-testid="ai-local-note" i18n="@@ai.setup.localNote">
      The assistant runs <strong>fully on this computer</strong>. Nothing about your logs leaves it:
      only the model file is downloaded, once, from Hugging Face (and, if none is installed yet, the
      inference runtime from llama.cpp's GitHub releases).
    </p>
    @switch (state().kind) {
      @case ('loading') {
        <p class="muted" role="status" i18n="@@ai.setup.checking">Checking this computer…</p>
      }
      @case ('plan') {
        @if (plan(); as p) {
          <section aria-labelledby="ai-hw-title">
            <h3 id="ai-hw-title" i18n="@@ai.setup.thisComputer">This computer</h3>
            <ul class="hw" data-testid="ai-hardware">
              @for (line of hwLines(); track $index) {
                <li>{{ line }}</li>
              }
            </ul>
            <p class="muted">{{ p.recommendations.message }}</p>
          </section>
          <section aria-labelledby="ai-models-title">
            <h3 id="ai-models-title" i18n="@@ai.setup.chooseModel">Choose a model</h3>
            <div class="models" role="radiogroup" aria-labelledby="ai-models-title">
              @for (r of ranked(); track r.model.id) {
                <label
                  class="model"
                  [class.picked]="r.model.id === selectedId()"
                  [class.too-large]="r.assessment.fit === 'tooLarge'"
                  [attr.data-testid]="'ai-model-' + r.model.id"
                >
                  <input
                    type="radio"
                    name="ai-model"
                    [value]="r.model.id"
                    [checked]="r.model.id === selectedId()"
                    (change)="dispatch({ type: 'select', modelId: r.model.id })"
                  />
                  <span class="name">
                    {{ r.model.name }}
                    @if (r.model.id === p.recommendations.best?.model?.id) {
                      <span class="badge best" i18n="@@ai.setup.badge.best">Best</span>
                    } @else if (r.model.id === p.recommendations.fastest?.model?.id) {
                      <span class="badge fastest" i18n="@@ai.setup.badge.fastest">Fastest</span>
                    }
                    @if (r.installed) {
                      <span class="badge" i18n="@@ai.setup.badge.installed">Installed</span>
                    }
                  </span>
                  <span class="facts">
                    <span class="fit" [class]="'fit fit-' + r.assessment.fit">{{ fit(r) }}</span>
                    <span>{{ speed(r) }}</span>
                    <span>{{ size(r.model.sizeBytes) }}</span>
                    <span i18n="@@ai.setup.licence">Licence: {{ r.model.license }}</span>
                    <span>{{ toolCalling(r) }}</span>
                    <span data-testid="ai-model-vision">{{ vision(!!r.model.projector) }}</span>
                  </span>
                  <span class="fit-detail muted" [attr.data-testid]="'ai-fit-' + r.model.id">{{ numbers(r) }}</span>
                  @if (r.assessment.fit !== 'comfortable') {
                    <span class="fit-detail muted" [attr.data-testid]="'ai-fit-advice-' + r.model.id">{{ facts(r) }}{{ advice(r) }}</span>
                  }
                  @if (r.model.notes) {
                    <span class="notes muted">{{ r.model.notes }}</span>
                  }
                </label>
              }
            </div>
          </section>
          @if (picked(); as r) {
            <p class="summary" data-testid="ai-plan-summary">
              @if (r.installed) {
                <ng-container i18n="@@ai.setup.summary.installed">{{ r.model.name }} is already downloaded; it will be loaded and checked.</ng-container>
              } @else {
                <ng-container i18n="@@ai.setup.summary.download">{{ r.model.name }}: {{ size(r.model.sizeBytes) }} will be downloaded from huggingface.co/{{ r.model.hfRepo }} (licence {{ r.model.license }}).</ng-container>
              }
            </p>
          }
          @if (runtimeNote(); as n) {
            <p class="note" role="note" data-testid="ai-setup-runtime-note">{{ n }}</p>
          }
          @if (p.note) {
            <p class="note" role="note">{{ p.note }}</p>
          }
          @if (startError(); as e) {
            <p class="err" role="alert" data-testid="ai-setup-start-error">{{ e }}</p>
          }
          <div class="foot">
            <button type="button" class="btn ghost" (click)="close()" i18n="@@ai.setup.notNow">Not now</button>
            <button
              type="button"
              class="btn primary"
              data-testid="ai-setup-start"
              [disabled]="!startable() || starting()"
              (click)="start()"
            >
              {{ startLabel() }}
            </button>
          </div>
        }
      }
      @case ('failed') {
        @if (failed(); as f) {
          <p class="err" role="alert" data-testid="ai-setup-error">{{ f.message }}</p>
          <div class="foot">
            <button type="button" class="btn ghost" (click)="close()" i18n="@@ai.setup.close">Close</button>
            <button type="button" class="btn primary" (click)="retry()" i18n="@@ai.setup.retry">Retry</button>
          </div>
        }
      }
    }
    }
  `,
  styles: `
    :host {
      display: flex;
      flex-direction: column;
      gap: 12px;
    }
    h3 {
      margin: 0 0 6px;
      font-size: var(--fs-md);
    }
    p {
      margin: 0;
    }
    .local {
      color: var(--text-2);
      padding: 8px 10px;
      border: 1px solid var(--border);
      border-radius: 6px;
      background: var(--bg-1);
    }
    .pitwall {
      display: flex;
      flex-direction: column;
      gap: 8px;
    }
    .code {
      font-size: var(--fs-lg, 1.5em);
      font-weight: 600;
      letter-spacing: 0.15em;
      font-variant-numeric: tabular-nums;
    }
    .inline-actions {
      display: flex;
      gap: 8px;
      flex-wrap: wrap;
    }
    .hw {
      margin: 0 0 4px;
      padding-left: 18px;
      color: var(--text-2);
    }
    .models {
      display: flex;
      flex-direction: column;
      gap: 6px;
    }
    .model {
      display: grid;
      grid-template-columns: auto 1fr;
      gap: 2px 8px;
      padding: 8px;
      border: 1px solid var(--border);
      border-radius: 6px;
      cursor: pointer;
    }
    /* The radio sits in its own column; every line of text lines up in the second one, however many there are. */
    .model input {
      grid-column: 1;
      grid-row: 1;
      margin-top: 3px;
    }
    .model > :not(input) {
      grid-column: 2;
    }
    .model.picked {
      border-color: var(--accent);
    }
    .model.too-large {
      border-style: dashed;
    }
    .name {
      font-weight: 600;
      display: flex;
      gap: 6px;
      align-items: center;
      flex-wrap: wrap;
    }
    .facts {
      display: flex;
      flex-wrap: wrap;
      gap: 4px 12px;
      color: var(--text-2);
      font-size: var(--fs-sm);
    }
    .notes {
      font-size: var(--fs-sm);
    }
    .fit-comfortable::before,
    .fit-tight::before,
    .fit-tooLarge::before {
      content: '';
      display: inline-block;
      width: 8px;
      height: 8px;
      border-radius: 50%;
      margin-right: 4px;
    }
    .fit-comfortable::before {
      background: var(--ok);
    }
    .fit-tight::before {
      background: var(--lvl-warn);
    }
    .fit-tooLarge::before {
      background: var(--lvl-error);
    }
    .badge.best,
    .badge.fastest {
      font-weight: 600;
    }
    .summary {
      color: var(--text-2);
    }
    .note {
      color: var(--muted);
      font-size: var(--fs-sm);
    }
    .err {
      color: var(--danger-text, var(--lvl-error));
    }
    /* The buttons stay in view while the models scroll. */
    .foot {
      display: flex;
      justify-content: flex-end;
      gap: 8px;
      position: sticky;
      bottom: -16px;
      margin-bottom: -16px;
      padding: 10px 0 16px;
      background: var(--bg-2);
      border-top: 1px solid var(--border);
    }
  `,
})
export class AiSetupWizard implements OnInit {
  static readonly dialogId = 'ai-setup';
  private readonly backend = inject(AiBackend);
  private readonly status = inject(AiStatusStore);
  protected readonly runtimes = inject(AiRuntimeStore);
  private readonly ref = inject(DIALOG_REF, { optional: true });
  /** Preselect this catalog model (from the model manager's "Download another model"). */
  readonly modelId = input<string | null>(null);

  readonly state = signal<WizardState>(INITIAL);
  /** "Download a model for Debrief instead": shows the model list even though Pitwall is running. */
  protected readonly downloadInstead = signal(false);
  protected readonly connecting = signal(false);
  /** The user asked for Pitwall in this dialog: closes it once Pitwall is paired and the engine. */
  private readonly waitingForPitwall = signal(false);
  protected readonly pairingFailure = signal<string | null>(null);
  protected readonly pitwall = computed(() => this.status.status()?.engine.pitwall ?? null);
  protected readonly pitwallChoice = computed(() => this.pitwall()?.found === 'running' && !this.downloadInstead());
  /** Pitwall is the engine already (and the dialog was not waiting for it): says so instead of offering it. */
  protected readonly usingPitwall = computed(() => {
    const e = this.status.status()?.engine;
    return e?.active === 'pitwall' ? e.pitwall : null;
  });
  protected readonly pairingError = computed(() => this.pairingFailure() ?? this.pitwall()?.problem ?? null);
  protected readonly group = groupCode;
  protected readonly starting = signal(false);
  protected readonly startError = signal<string | null>(null);
  protected readonly plan = computed(() => (this.state().kind === 'plan' ? (this.state() as Extract<WizardState, { kind: 'plan' }>).plan : null));
  protected readonly ranked = computed(() => {
    const p = this.plan();
    return p ? choices(p) : [];
  });
  protected readonly selectedId = computed(() => {
    const s = this.state();
    return s.kind === 'plan' ? s.selectedId : null;
  });
  protected readonly picked = computed(() => selected(this.state()));
  protected readonly startable = computed(() => canStart(this.state()));
  protected readonly hwLines = computed(() => {
    const p = this.plan();
    return p ? hardwareLines(p.hardware) : [];
  });
  protected readonly failed = computed(() => {
    const s = this.state();
    return s.kind === 'failed' ? s : null;
  });
  /** Says that the inference runtime comes first when none is installed (it is part of the same background job). */
  protected readonly runtimeNote = computed(() => {
    const r = this.runtimes.suggested();
    return this.runtimes.needsDownload() && r
      ? $localize`:@@ai.setup.runtimeNote:The inference runtime (${r.label}:build:, ${formatBytes(r.downloadBytes)}:size:) is downloaded first.`
      : null;
  });

  protected readonly fit = (r: Recommendation) => fitLabel(r.assessment.fit);
  protected readonly numbers = (r: Recommendation) => fitNumbers(r.assessment);
  protected readonly facts = (r: Recommendation) => {
    const f = fitFacts(r.assessment);
    return `${f.machine} · ${f.now}`;
  };
  protected readonly advice = (r: Recommendation) => {
    const a = fitAdvice(r.assessment);
    return a.length ? ` · ${a.join(' · ')}` : '';
  };
  protected readonly speed = (r: Recommendation) => speedLabel(r.assessment.speed);
  protected readonly size = formatBytes;
  protected readonly startLabel = computed(() =>
    this.picked()?.installed ? $localize`:@@ai.setup.start.installed:Set up` : $localize`:@@ai.setup.start.download:Download and set up`,
  );

  protected readonly vision = visionLabel;

  protected toolCalling(r: Recommendation): string {
    return r.model.toolCalling ? $localize`:@@ai.setup.toolCalling:Tool calling` : $localize`:@@ai.setup.noToolCalling:No tool calling`;
  }

  constructor() {
    // Closing the dialog is a side effect outside the signal graph: done once Pitwall accepted the pairing.
    effect(() => {
      const e = this.status.status()?.engine;
      if (this.waitingForPitwall() && e?.pitwall.paired && e.active === 'pitwall') this.close();
    });
  }

  /** "Use Pitwall": pairs (or just chooses it when already paired); the dialog closes when Pitwall is the engine. */
  async usePitwall(): Promise<void> {
    if (this.connecting()) return;
    this.connecting.set(true);
    this.pairingFailure.set(null);
    this.waitingForPitwall.set(true);
    try {
      const result = await this.backend.pitwallConnect(true);
      if (result.usable) this.close();
    } catch (e) {
      this.waitingForPitwall.set(false);
      this.pairingFailure.set(toAiError(e).message);
    } finally {
      this.connecting.set(false);
    }
  }

  async cancelPitwall(): Promise<void> {
    this.waitingForPitwall.set(false);
    try {
      await this.backend.pitwallCancel();
    } catch (e) {
      this.pairingFailure.set(toAiError(e).message);
    }
  }

  ngOnInit(): void {
    void this.load();
  }

  dispatch(e: WizardEvent): void {
    this.state.update((s) => next(s, e));
  }

  async load(): Promise<void> {
    try {
      const [plan] = await Promise.all([this.backend.setupPlan(this.modelId()), this.runtimes.load()]);
      this.dispatch({ type: 'planLoaded', plan, preferred: this.modelId() });
      void this.refineFit();
    } catch (e) {
      const err = toAiError(e);
      this.dispatch({ type: 'failed', code: err.code, message: err.message });
    }
  }

  /** Replaces the rough memory figures with exact ones once the engine has read the model headers. */
  private async refineFit(): Promise<void> {
    try {
      this.dispatch({ type: 'fitRefined', recommendations: await this.backend.refineFit() });
    } catch {
      // Offline or the host is down: the rough figures stay (and say so).
    }
  }

  /** Hands the choice to the engine and closes: the rest is a background job (Jobs, status bar, a message at the end). */
  async start(): Promise<void> {
    const id = this.selectedId();
    if (!id || !this.startable() || this.starting()) return;
    this.starting.set(true);
    this.startError.set(null);
    try {
      await this.backend.setupBackground(id);
      void this.status.refresh();
      this.close();
    } catch (e) {
      this.startError.set(toAiError(e).message);
      this.starting.set(false);
    }
  }

  retry(): void {
    this.dispatch({ type: 'retry' });
    void this.load();
  }

  close(): void {
    this.ref?.close();
  }
}
