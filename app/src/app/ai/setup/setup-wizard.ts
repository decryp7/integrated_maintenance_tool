import { transferSpeed } from '../../shared/format/transfer-speed';
import {
  ChangeDetectionStrategy,
  Component,
  computed,
  DestroyRef,
  inject,
  input,
  OnInit,
  signal,
} from '@angular/core';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { DIALOG_REF } from '../../ui/dialog.service';
import { AiBackend, toAiError } from '../ai-backend';
import {
  fitAdvice,
  fitFacts,
  fitLabel,
  fitNumbers,
  formatBytes,
  formatEta,
  formatRate,
  hardwareLines,
  percent,
  speedLabel,
  visionLabel,
} from '../ai-format';
import { AiStatusStore } from '../ai-status.store';
import { Recommendation, SetupEvent } from '../ai.types';
import { AiRuntimeStore } from '../runtimes/ai-runtime.store';
import { AiManualDownload } from '../manual/manual-download';
import { offersManualDownload } from '../manual/manual-download-view';
import { AiRuntimePanel } from '../runtimes/runtime-panel';
import {
  canStart,
  choices,
  INITIAL,
  next,
  selected,
  stageLabel,
  stageStep,
  WizardEvent,
  WizardState,
} from './wizard';

/**
 * "Set up AI assistant": downloads an inference runtime when none is installed, shows this computer's
 * hardware and the models that fit it, downloads and verifies the chosen one, loads it (GPU first, CPU fallback) and warms it up. Opened in a dialog.
 */
@Component({
  selector: 'debrief-ai-setup-wizard',
  imports: [AiManualDownload, AiRuntimePanel],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p class="local" data-testid="ai-local-note" i18n="@@ai.setup.localNote">
      The assistant runs <strong>fully on this computer</strong>. Nothing about your logs leaves it:
      only the model file is downloaded, once, from Hugging Face (and, if none is installed yet, the
      inference runtime from llama.cpp's GitHub releases).
    </p>
    <div class="sr-only" aria-live="polite" data-testid="ai-setup-live">{{ liveText() }}</div>
    @switch (state().kind) {
      @case ('loading') {
        <p class="muted" role="status" i18n="@@ai.setup.checking">Checking this computer…</p>
      }
      @case ('runtime') {
        <section aria-labelledby="ai-runtime-title" data-testid="ai-setup-runtime">
          <h3 id="ai-runtime-title" i18n="@@ai.models.runtime">Inference runtime</h3>
          <p class="muted" i18n="@@ai.setup.runtime.intro">
            First, download the engine that runs the model on this computer. The suggested build
            suits its hardware; you can switch later in the model manager.
          </p>
          <debrief-ai-runtime-panel />
        </section>
        <div class="foot">
          <button type="button" class="btn ghost" (click)="close()" i18n="@@ai.setup.notNow">Not now</button>
          <button
            type="button"
            class="btn primary"
            data-testid="ai-runtime-continue"
            [disabled]="runtimes.needsDownload() || runtimes.installing()"
            (click)="dispatch({ type: 'runtimeReady' })"
            i18n="@@ai.setup.continue"
          >
            Continue
          </button>
        </div>
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
          @if (p.note) {
            <p class="note" role="note">{{ p.note }}</p>
          }
          <div class="foot">
            <button type="button" class="btn ghost" (click)="close()" i18n="@@ai.setup.notNow">Not now</button>
            <button
              type="button"
              class="btn primary"
              data-testid="ai-setup-start"
              [disabled]="!startable()"
              (click)="start()"
            >
              {{ startLabel() }}
            </button>
          </div>
        }
      }
      @case ('running') {
        @if (running(); as r) {
          <p class="step" data-testid="ai-setup-stage" i18n="@@ai.setup.step">Step {{ step() }} of 5 · {{ label() }}</p>
          @if (r.stage.stage === 'downloading' || r.stage.stage === 'verifying') {
            <div
              class="bar"
              role="progressbar"
              [attr.aria-label]="progressAria()"
              aria-valuemin="0"
              aria-valuemax="100"
              [attr.aria-valuenow]="pct()"
            >
              <span class="fill" [style.width.%]="pct()"></span>
            </div>
            <p class="muted" data-testid="ai-setup-bytes">{{ progressText() }}</p>
          }
          @if (r.note) {
            <p class="note" role="note" data-testid="ai-setup-fallback">{{ r.note }}</p>
          }
          <div class="foot">
            <button
              type="button"
              class="btn"
              data-testid="ai-setup-cancel"
              [disabled]="r.cancelling"
              (click)="cancel()"
            >
              {{ cancelLabel(r.cancelling) }}
            </button>
          </div>
        }
      }
      @case ('ready') {
        @if (ready(); as r) {
          <p class="ok" role="status" data-testid="ai-setup-ready">
            <ng-container i18n="@@ai.setup.ready">Ready: {{ r.model }} runs on {{ r.backend }}.</ng-container>
          </p>
          @if (r.note) {
            <p class="note" role="note">{{ r.note }}</p>
          }
          <div class="foot">
            <button type="button" class="btn ghost" (click)="close()" i18n="@@ai.setup.close">Close</button>
            <button
              type="button"
              class="btn primary"
              data-testid="ai-open-chat"
              (click)="openChat()"
              i18n="@@ai.setup.openChat"
            >
              Open the assistant
            </button>
          </div>
        }
      }
      @case ('failed') {
        @if (failed(); as f) {
          <p class="err" role="alert" data-testid="ai-setup-error">{{ f.message }}</p>
          @if (manualModel(); as id) {
            <debrief-ai-manual-download kind="model" [id]="id" [code]="f.code" (added)="retry()" />
          }
          @if (f.code === 'noRuntime') {
            <section aria-labelledby="ai-runtime-fix-title" data-testid="ai-setup-runtime">
              <h3 id="ai-runtime-fix-title" i18n="@@ai.models.runtime">Inference runtime</h3>
              <debrief-ai-runtime-panel />
            </section>
          }
          <div class="foot">
            <button type="button" class="btn ghost" (click)="close()" i18n="@@ai.setup.close">Close</button>
            <button type="button" class="btn primary" (click)="retry()" i18n="@@ai.setup.retry">Retry</button>
          </div>
        }
      }
      @case ('cancelled') {
        <p role="status" data-testid="ai-setup-cancelled" i18n="@@ai.setup.cancelled">
          The set-up was cancelled. Nothing was changed.
        </p>
        <div class="foot">
          <button type="button" class="btn ghost" (click)="close()" i18n="@@ai.setup.close">Close</button>
          <button type="button" class="btn primary" (click)="retry()" i18n="@@ai.setup.startAgain">Start again</button>
        </div>
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
    .bar {
      height: 8px;
      border-radius: 4px;
      background: var(--bg-2);
      overflow: hidden;
    }
    .fill {
      display: block;
      height: 100%;
      background: var(--accent);
      transition: width 0.2s;
    }
    .ok {
      color: var(--ok);
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
  private readonly ctx = inject(COMMAND_CONTEXT);
  private readonly ref = inject(DIALOG_REF, { optional: true });
  private readonly destroyRef = inject(DestroyRef);
  /** Preselect this catalog model (from the model manager's "Download another model"). */
  readonly modelId = input<string | null>(null);

  readonly state = signal<WizardState>(INITIAL);
  protected readonly plan = computed(() =>
    this.state().kind === 'plan'
      ? (this.state() as Extract<WizardState, { kind: 'plan' }>).plan
      : null,
  );
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
  protected readonly running = computed(() => {
    const s = this.state();
    return s.kind === 'running' ? s : null;
  });
  protected readonly ready = computed(() => {
    const s = this.state();
    return s.kind === 'ready' ? s : null;
  });
  protected readonly failed = computed(() => {
    const s = this.state();
    return s.kind === 'failed' ? s : null;
  });
  /** The model whose download failed in a way a manual download works around (a proxy, no network). */
  protected readonly manualModel = computed(() => {
    const f = this.failed();
    return f?.modelId && offersManualDownload(f.code) ? f.modelId : null;
  });
  protected readonly step = computed(() => {
    const r = this.running();
    return r ? stageStep(r.stage) : 0;
  });
  protected readonly label = computed(() => {
    const r = this.running();
    return r ? stageLabel(r.stage) : '';
  });
  protected readonly pct = computed(() => {
    const st = this.running()?.stage;
    return st && (st.stage === 'downloading' || st.stage === 'verifying')
      ? percent(st.bytes, st.total)
      : 0;
  });
  private readonly transfer = transferSpeed(() => {
    const st = this.running()?.stage;
    return st?.stage === 'downloading' ? st : null;
  });
  protected readonly progressText = computed(() => {
    const st = this.running()?.stage;
    if (!st || (st.stage !== 'downloading' && st.stage !== 'verifying')) return '';
    const base = $localize`:@@ai.runtime.progress.of:${formatBytes(st.bytes)}:done: of ${formatBytes(st.total)}:total:`;
    if (st.stage === 'verifying') return $localize`:@@ai.runtime.progress.checking:Checking the download: ${base}:progress:`;
    return [
      base,
      formatRate(this.transfer().rate),
      formatEta(this.transfer().etaSecs),
    ]
      .filter(Boolean)
      .join(' · ');
  });
  /** Announced to screen readers when the stage changes. */
  protected readonly liveText = computed(() => {
    const s = this.state();
    switch (s.kind) {
      case 'running':
        return stageLabel(s.stage);
      case 'ready':
        return $localize`:@@ai.setup.live.ready:The AI assistant is ready.`;
      case 'failed':
        return $localize`:@@ai.setup.live.failed:The set-up failed: ${s.message}:message:`;
      case 'cancelled':
        return $localize`:@@ai.setup.live.cancelled:The set-up was cancelled.`;
      default:
        return '';
    }
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
  protected readonly progressAria = computed(() =>
    this.running()?.stage.stage === 'downloading'
      ? $localize`:@@ai.setup.progress.download:Download progress`
      : $localize`:@@ai.setup.progress.verify:Verification progress`,
  );

  protected readonly vision = visionLabel;

  protected toolCalling(r: Recommendation): string {
    return r.model.toolCalling ? $localize`:@@ai.setup.toolCalling:Tool calling` : $localize`:@@ai.setup.noToolCalling:No tool calling`;
  }

  protected cancelLabel(cancelling: boolean): string {
    return cancelling ? $localize`:@@ai.runtime.cancelling:Cancelling…` : $localize`:@@ai.setup.cancel:Cancel`;
  }

  ngOnInit(): void {
    let off: (() => void) | null = null;
    let destroyed = false;
    this.destroyRef.onDestroy(() => {
      destroyed = true;
      off?.();
    });
    void this.backend
      .onSetup((e) => this.onSetup(e))
      .then((u) => (destroyed ? u() : (off = u)))
      .catch(() => undefined);
    void this.load();
  }

  dispatch(e: WizardEvent): void {
    this.state.update((s) => next(s, e));
  }

  private onSetup(e: SetupEvent): void {
    this.dispatch({ type: 'setup', event: e });
    if (e.stage === 'ready') void this.status.refresh();
  }

  async load(): Promise<void> {
    try {
      const [plan, runtimes] = await Promise.all([
        this.backend.setupPlan(this.modelId()),
        this.runtimes.load(),
      ]);
      this.dispatch({
        type: 'planLoaded',
        plan,
        preferred: this.modelId(),
        needsRuntime: !!runtimes?.needsDownload,
      });
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

  async start(): Promise<void> {
    const id = this.selectedId();
    if (!id || !this.startable()) return;
    this.dispatch({ type: 'start' });
    try {
      await this.backend.setupRun(id);
    } catch (e) {
      const err = toAiError(e);
      this.dispatch({ type: 'failed', code: err.code, message: err.message });
    }
  }

  async cancel(): Promise<void> {
    this.dispatch({ type: 'cancelRequested' });
    try {
      await this.backend.setupCancel();
    } catch {
      /* the stage event still ends the set-up */
    }
  }

  retry(): void {
    this.dispatch({ type: 'retry' });
    void this.load();
  }

  openChat(): void {
    this.ctx.showView('ai-chat');
    this.close();
  }

  close(): void {
    this.ref?.close();
  }
}
