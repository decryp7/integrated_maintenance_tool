import {
  ChangeDetectionStrategy,
  Component,
  computed,
  DestroyRef,
  inject,
  OnInit,
  signal,
} from '@angular/core';
import { Platform } from '../../core/platform';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { DialogService } from '../../ui/dialog.service';
import { AiBackend, toAiError } from '../ai-backend';
import { fitLabel, formatBytes, visionLabel } from '../ai-format';
import { AiStatusStore } from '../ai-status.store';
import { InstalledModel, Recommendation, SetupEvent } from '../ai.types';
import { AiRuntimePanel } from '../runtimes/runtime-panel';
import { AiSetupWizard } from '../setup/setup-wizard';
import { stageLabel, RunningStage } from '../setup/wizard';
import { JobFollow } from '../../jobs/job-follow';

const ENGINE_STATE: Record<string, string> = {
  unavailable: $localize`:@@ai.models.state.unavailable:Unavailable`,
  disabled: $localize`:@@ai.models.state.disabled:Switched off after repeated failures`,
  noModel: $localize`:@@ai.models.state.noModel:No model set up`,
  idle: $localize`:@@ai.models.state.idle:Ready (loads on first use)`,
  loading: $localize`:@@ai.models.state.loading:Loading the model…`,
  ready: $localize`:@@ai.models.state.ready:Running`,
  failed: $localize`:@@ai.models.state.failed:The last load failed`,
};

/** The engine status, the inference runtimes, installed models, adding a local GGUF file and downloading other models. */
@Component({
  selector: 'debrief-ai-model-manager',
  imports: [AiRuntimePanel],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <section class="engine" aria-labelledby="ai-engine-title" data-testid="ai-engine-status">
      <h3 id="ai-engine-title" i18n="@@ai.models.engine">Engine</h3>
      @if (status.status(); as s) {
        <p>
          {{ engineState() }}
          @if (s.backend) {
            · {{ s.backend }}
          }
          @if (s.rssBytes) {
            <ng-container i18n="@@ai.models.inMemory">· {{ size(s.rssBytes) }} in memory</ng-container>
          }
        </p>
        @if (s.reason) {
          <p class="err" role="alert">{{ s.reason }}</p>
        }
        @if (s.state === 'disabled' || s.state === 'failed') {
          <button
            type="button"
            class="btn"
            data-testid="ai-engine-reset"
            [disabled]="busy()"
            (click)="reset()"
            i18n="@@ai.models.resetEngine"
          >
            Reset engine
          </button>
        }
      } @else {
        <p class="muted" i18n="@@ai.models.engineUnknown">Engine status unknown.</p>
      }
    </section>

    <section aria-labelledby="ai-runtime-title" data-testid="ai-runtime-section">
      <h3 id="ai-runtime-title" i18n="@@ai.models.runtime">Inference runtime</h3>
      <debrief-ai-runtime-panel />
    </section>

    <section aria-labelledby="ai-installed-title">
      <div class="head">
        <h3 id="ai-installed-title" i18n="@@ai.models.installed">Installed models</h3>
        <button
          type="button"
          class="btn"
          data-testid="ai-add-local"
          [disabled]="busy()"
          (click)="addLocal()"
          i18n="@@ai.models.addLocal"
        >
          Add local GGUF…
        </button>
      </div>
      <div class="sr-only" aria-live="polite">{{ progress() }}</div>
      @if (progress()) {
        <p class="muted" role="status" data-testid="ai-model-progress">{{ progress() }}</p>
      }
      @if (error()) {
        <p class="err" role="alert" data-testid="ai-models-error">
          {{ error() }}
          <button type="button" class="btn ghost" (click)="load()" i18n="@@ai.models.retry">Retry</button>
        </p>
      }
      @if (models().length) {
        <ul class="list">
          @for (m of models(); track m.fileName) {
            <li class="row" [attr.data-testid]="'ai-installed-' + m.fileName">
              <div class="main">
                <span class="name">
                  {{ m.name }}
                  @if (m.active) {
                    <span class="badge active" i18n="model in use|@@ai.models.badge.active">Active</span>
                  }
                  @if (m.assessment; as a) {
                    <span class="badge" [class]="'badge fit-' + a.fit">{{ fit(a.fit) }}</span>
                  }
                </span>
                <span class="facts mono"
                  >{{ m.fileName }} · {{ size(m.sizeBytes) }}
                  @if (m.license) {
                    · {{ m.license }}
                  }
                  @if (m.toolCalling) {
                    <ng-container i18n="@@ai.models.toolCalling"> · tool calling</ng-container>
                  }
                </span>
                <span class="facts" data-testid="ai-model-vision">{{ visionLabel(m.vision) }}@if (m.projector; as p) { · {{ p.fileName }} ({{ size(p.sizeBytes) }})}</span>
                @if (m.problem) {
                  <span class="err">{{ m.problem }}</span>
                }
                @if (m.projector?.problem; as pp) {
                  <span class="err">{{ pp }}</span>
                }
              </div>
              <div class="actions">
                @if (!m.vision && m.projectorDownloadBytes !== null) {
                  <button
                    type="button"
                    class="btn"
                    data-testid="ai-add-image-support"
                    [disabled]="busy() || projectorRunning() === m.fileName"
                    (click)="addImageSupport(m)"
                    i18n="@@ai.vision.models.addSupport"
                  >
                    Add image support ({{ size(m.projectorDownloadBytes) }})
                  </button>
                }
                @if (!m.vision) {
                  <button
                    type="button"
                    class="btn ghost"
                    data-testid="ai-add-projector"
                    [disabled]="busy()"
                    (click)="addProjectorFile(m)"
                    [attr.aria-label]="addProjectorLabel(m)"
                    i18n="@@ai.vision.models.addProjector"
                  >
                    Add projector file…
                  </button>
                }
                @if (!m.active) {
                  <button
                    type="button"
                    class="btn"
                    [disabled]="busy() || !!m.problem"
                    (click)="activate(m)"
                    [attr.aria-label]="useLabel(m)"
                    i18n="verb, button that selects a model|@@ai.models.use"
                  >
                    Use
                  </button>
                }
                <button
                  type="button"
                  class="btn ghost danger"
                  [disabled]="busy()"
                  (click)="remove(m)"
                  [attr.aria-label]="deleteLabel(m)"
                  i18n="@@ai.models.delete"
                >
                  Delete
                </button>
              </div>
            </li>
          }
        </ul>
      } @else if (loaded()) {
        <p class="muted" data-testid="ai-no-models" i18n="@@ai.models.none">No model is installed yet.</p>
      }
    </section>

    <section aria-labelledby="ai-catalog-title">
      <h3 id="ai-catalog-title" i18n="@@ai.models.catalog">Download another model</h3>
      <ul class="list">
        @for (r of others(); track r.model.id) {
          <li class="row">
            <div class="main">
              <span class="name"
                >{{ r.model.name }}
                <span class="badge" [class]="'badge fit-' + r.assessment.fit">{{
                  fit(r.assessment.fit)
                }}</span></span
              >
              <span class="facts"
                >{{ size(r.model.sizeBytes) }} · {{ r.model.license
                }}@if (r.model.toolCalling) {<ng-container i18n="@@ai.models.toolCalling"> · tool calling</ng-container>} · {{ visionLabel(!!r.model.projector) }}</span
              >
              @if (r.model.notes) {
                <span class="facts notes" data-testid="ai-catalog-notes">{{ r.model.notes }}</span>
              }
            </div>
            <div class="actions">
              <button
                type="button"
                class="btn"
                [disabled]="busy() || r.assessment.fit === 'tooLarge'"
                (click)="download(r)"
                [attr.aria-label]="downloadLabel(r)"
                i18n="@@ai.models.download"
              >
                Download…
              </button>
            </div>
          </li>
        }
      </ul>
    </section>
  `,
  styles: `
    :host {
      display: flex;
      flex-direction: column;
      gap: 14px;
    }
    h3 {
      margin: 0 0 6px;
      font-size: var(--fs-md);
    }
    p {
      margin: 0 0 4px;
    }
    .head {
      display: flex;
      justify-content: space-between;
      align-items: center;
    }
    .list {
      list-style: none;
      margin: 0;
      padding: 0;
      display: flex;
      flex-direction: column;
      gap: 6px;
      max-height: 260px;
      overflow: auto;
    }
    .row {
      display: flex;
      gap: 10px;
      align-items: center;
      padding: 8px;
      border: 1px solid var(--border);
      border-radius: 6px;
    }
    .main {
      display: flex;
      flex-direction: column;
      gap: 2px;
      flex: 1;
      min-width: 0;
    }
    .name {
      font-weight: 600;
      display: flex;
      gap: 6px;
      align-items: center;
      flex-wrap: wrap;
    }
    .facts {
      font-size: var(--fs-sm);
      color: var(--text-2);
      overflow-wrap: anywhere;
    }
    .notes {
      color: var(--muted);
    }
    .actions {
      display: flex;
      gap: 6px;
    }
    .badge.active {
      font-weight: 600;
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
    .danger {
      color: var(--danger-text, var(--lvl-error));
    }
    .err {
      color: var(--danger-text, var(--lvl-error));
    }
  `,
})
export class AiModelManager implements OnInit {
  static readonly dialogId = 'ai-models';
  private readonly backend = inject(AiBackend);
  private readonly dialogs = inject(DialogService);
  private readonly platform = inject(Platform);
  private readonly destroyRef = inject(DestroyRef);
  protected readonly status = inject(AiStatusStore);

  readonly models = signal<InstalledModel[]>([]);
  readonly catalog = signal<Recommendation[]>([]);
  readonly loaded = signal(false);
  readonly busy = signal(false);
  readonly error = signal<string | null>(null);
  readonly progress = signal('');
  /** The model whose image support is downloading. */
  readonly projectorRunning = signal<string | null>(null);
  private readonly follow = inject(JobFollow);
  protected readonly visionLabel = visionLabel;

  protected readonly others = computed(() => {
    const have = new Set(this.models().map((m) => m.fileName));
    return this.catalog().filter((r) => !have.has(r.model.file));
  });
  protected readonly engineState = computed(
    () => ENGINE_STATE[this.status.status()?.state ?? ''] ?? '',
  );
  protected readonly size = formatBytes;
  protected readonly fit = fitLabel;

  protected useLabel(m: InstalledModel): string {
    return $localize`:@@ai.models.use.label:Use ${m.name}:name:`;
  }

  protected deleteLabel(m: InstalledModel): string {
    return $localize`:@@ai.models.delete.label:Delete ${m.name}:name:`;
  }

  protected downloadLabel(r: Recommendation): string {
    return $localize`:@@ai.models.download.label:Download ${r.model.name}:name:`;
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
    void this.status.refresh();
    void this.load();
  }

  async load(): Promise<void> {
    this.error.set(null);
    try {
      const [models, catalog] = await Promise.all([this.backend.models(), this.backend.catalog()]);
      this.models.set(models);
      this.catalog.set(catalog);
    } catch (e) {
      this.error.set(toAiError(e).message);
    } finally {
      this.loaded.set(true);
    }
  }

  private onSetup(e: SetupEvent): void {
    if (!this.busy()) return;
    if (e.stage === 'ready' || e.stage === 'failed' || e.stage === 'cancelled') {
      this.busy.set(false);
      this.progress.set(e.stage === 'ready' ? $localize`:@@ai.models.nowActive:${e.model}:model: is now active.` : '');
      void this.status.refresh();
      void this.load().then(() => {
        if (e.stage === 'failed') this.error.set(e.message);
      });
    } else if (e.stage === 'fellBackToCpu') {
      this.progress.set(e.reason);
    } else {
      this.progress.set(stageLabel(e as RunningStage));
    }
  }

  async activate(m: InstalledModel): Promise<void> {
    this.busy.set(true);
    this.error.set(null);
    this.progress.set($localize`:@@ai.models.loading:Loading ${m.name}:name:…`);
    try {
      await this.backend.activateModel(m.fileName);
    } catch (e) {
      this.busy.set(false);
      this.progress.set('');
      this.error.set(toAiError(e).message);
    }
  }

  async remove(m: InstalledModel): Promise<void> {
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:@@ai.models.confirm.title:Delete model?`,
      width: 460,
      inputs: {
        message: m.active
          ? $localize`:@@ai.models.confirm.deleteActive:Delete ${m.name}:name:? The file (${formatBytes(m.sizeBytes)}:size:) is removed from this computer and the assistant has no model until you choose another.`
          : $localize`:@@ai.models.confirm.delete:Delete ${m.name}:name:? The file (${formatBytes(m.sizeBytes)}:size:) is removed from this computer.`,
        detail: m.fileName,
        confirmLabel: $localize`:@@ai.models.delete.confirm:Delete`,
        danger: true,
      },
    });
    if ((await ref.closed) !== true) return;
    this.busy.set(true);
    this.error.set(null);
    try {
      await this.backend.deleteModel(m.fileName);
      await this.status.refresh();
      await this.load();
    } catch (e) {
      this.error.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
    }
  }

  async addLocal(): Promise<void> {
    const path = await this.platform.pickOpenPath($localize`:@@ai.models.addLocal.dialogTitle:Add a GGUF model`, [
      { name: $localize`:@@ai.models.addLocal.filter:GGUF model`, extensions: ['gguf'] },
    ]);
    if (!path) return;
    this.busy.set(true);
    this.error.set(null);
    try {
      const m = await this.backend.addLocalModel(path);
      this.progress.set($localize`:@@ai.models.added:${m.name}:name: was added. Choose Use to make it active.`);
      await this.load();
    } catch (e) {
      this.error.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
    }
  }

  protected addProjectorLabel(m: InstalledModel): string {
    return $localize`:@@ai.vision.models.addProjectorLabel:Add a projector file for ${m.name}:name:`;
  }

  /** "Add image support": the catalog projector as a download job (only when the user asks). */
  async addImageSupport(m: InstalledModel): Promise<void> {
    this.error.set(null);
    this.projectorRunning.set(m.fileName);
    try {
      const id = await this.backend.installProjector(m.fileName);
      this.progress.set($localize`:@@ai.vision.models.downloading:Downloading image support for ${m.name}:name: — running in Jobs`);
      const end = await this.follow.follow(id, this.destroyRef);
      if (!end) return;
      await this.status.refresh();
      await this.load();
      if (end.state === 'failed') this.error.set(end.error ?? $localize`:@@ai.vision.projector.failed:The download failed.`);
      else if (end.state === 'done') this.progress.set($localize`:@@ai.vision.models.added:${m.name}:name: now reads images.`);
    } catch (e) {
      this.error.set(toAiError(e).message);
    } finally {
      this.projectorRunning.set(null);
    }
  }

  /** A projector file the user chose for an installed model (the engine checks it fits). */
  async addProjectorFile(m: InstalledModel): Promise<void> {
    const path = await this.platform.pickOpenPath($localize`:@@ai.vision.models.pickProjector:Add an image projector (mmproj GGUF)`, [
      { name: $localize`:@@ai.vision.models.projectorFilter:Image projector`, extensions: ['gguf'] },
    ]);
    if (!path) return;
    this.busy.set(true);
    this.error.set(null);
    try {
      const added = await this.backend.addProjector(m.fileName, path);
      this.progress.set($localize`:@@ai.vision.models.added:${added.name}:name: now reads images.`);
      await this.status.refresh();
      await this.load();
    } catch (e) {
      this.error.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
    }
  }

  async reset(): Promise<void> {
    this.busy.set(true);
    try {
      this.status.setStatus(await this.backend.reset());
    } catch (e) {
      this.error.set(toAiError(e).message);
    } finally {
      this.busy.set(false);
    }
  }

  download(r: Recommendation): void {
    this.dialogs.open(AiSetupWizard, {
      title: $localize`:@@ai.models.setupTitle:Set up ${r.model.name}:name:`,

      width: 760,
      inputs: { modelId: r.model.id },
    });
  }
}
