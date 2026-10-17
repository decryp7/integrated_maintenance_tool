import { ChangeDetectionStrategy, Component, computed, DestroyRef, inject, OnInit, signal } from '@angular/core';
import { Platform } from '../../core/platform';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { DialogService } from '../../ui/dialog.service';
import { AiBackend, toAiError } from '../ai-backend';
import { fitLabel, formatBytes } from '../ai-format';
import { AiStatusStore } from '../ai-status.store';
import { InstalledModel, Recommendation, SetupEvent } from '../ai.types';
import { AiSetupWizard } from '../setup/setup-wizard';
import { stageLabel, RunningStage } from '../setup/wizard';

const ENGINE_STATE: Record<string, string> = {
  unavailable: 'Unavailable',
  disabled: 'Switched off after repeated failures',
  noModel: 'No model set up',
  idle: 'Ready (loads on first use)',
  loading: 'Loading the model…',
  ready: 'Running',
  failed: 'The last load failed',
};

/** Installed models, the engine status, adding a local GGUF file and downloading other models. */
@Component({
  selector: 'imt-ai-model-manager',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <section class="engine" aria-labelledby="ai-engine-title" data-testid="ai-engine-status">
      <h3 id="ai-engine-title">Engine</h3>
      @if (status.status(); as s) {
        <p>
          {{ engineState() }}
          @if (s.backend) {
            · {{ s.backend }}
          }
          @if (s.rssBytes) {
            · {{ size(s.rssBytes) }} in memory
          }
        </p>
        @if (s.reason) {
          <p class="err" role="alert">{{ s.reason }}</p>
        }
        @if (s.state === 'disabled' || s.state === 'failed') {
          <button type="button" class="btn" data-testid="ai-engine-reset" [disabled]="busy()" (click)="reset()">Reset engine</button>
        }
      } @else {
        <p class="muted">Engine status unknown.</p>
      }
    </section>

    <section aria-labelledby="ai-installed-title">
      <div class="head">
        <h3 id="ai-installed-title">Installed models</h3>
        <button type="button" class="btn" data-testid="ai-add-local" [disabled]="busy()" (click)="addLocal()">Add local GGUF…</button>
      </div>
      <div class="sr-only" aria-live="polite">{{ progress() }}</div>
      @if (progress()) {
        <p class="muted" role="status" data-testid="ai-model-progress">{{ progress() }}</p>
      }
      @if (error()) {
        <p class="err" role="alert" data-testid="ai-models-error">
          {{ error() }}
          <button type="button" class="btn ghost" (click)="load()">Retry</button>
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
                    <span class="badge active">Active</span>
                  }
                  @if (m.assessment; as a) {
                    <span class="badge" [class]="'badge fit-' + a.fit">{{ fit(a.fit) }}</span>
                  }
                </span>
                <span class="facts mono">{{ m.fileName }} · {{ size(m.sizeBytes) }}@if (m.license) { · {{ m.license }}}@if (m.toolCalling) { · tool calling}</span>
                @if (m.problem) {
                  <span class="err">{{ m.problem }}</span>
                }
              </div>
              <div class="actions">
                @if (!m.active) {
                  <button type="button" class="btn" [disabled]="busy() || !!m.problem" (click)="activate(m)" [attr.aria-label]="'Use ' + m.name">Use</button>
                }
                <button type="button" class="btn ghost danger" [disabled]="busy()" (click)="remove(m)" [attr.aria-label]="'Delete ' + m.name">Delete</button>
              </div>
            </li>
          }
        </ul>
      } @else if (loaded()) {
        <p class="muted" data-testid="ai-no-models">No model is installed yet.</p>
      }
    </section>

    <section aria-labelledby="ai-catalog-title">
      <h3 id="ai-catalog-title">Download another model</h3>
      <ul class="list">
        @for (r of others(); track r.model.id) {
          <li class="row">
            <div class="main">
              <span class="name">{{ r.model.name }} <span class="badge" [class]="'badge fit-' + r.assessment.fit">{{ fit(r.assessment.fit) }}</span></span>
              <span class="facts">{{ size(r.model.sizeBytes) }} · {{ r.model.license }}{{ r.model.toolCalling ? ' · tool calling' : '' }}</span>
            </div>
            <div class="actions">
              <button type="button" class="btn" [disabled]="busy() || r.assessment.fit === 'tooLarge'" (click)="download(r)" [attr.aria-label]="'Download ' + r.model.name">
                Download…
              </button>
            </div>
          </li>
        }
      </ul>
    </section>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 14px; }
    h3 { margin: 0 0 6px; font-size: 13px; }
    p { margin: 0 0 4px; }
    .head { display: flex; justify-content: space-between; align-items: center; }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; max-height: 260px; overflow: auto; }
    .row { display: flex; gap: 10px; align-items: center; padding: 8px; border: 1px solid var(--border); border-radius: 6px; }
    .main { display: flex; flex-direction: column; gap: 2px; flex: 1; min-width: 0; }
    .name { font-weight: 600; display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
    .facts { font-size: 12px; color: var(--text-2); overflow-wrap: anywhere; }
    .actions { display: flex; gap: 6px; }
    .badge.active { font-weight: 600; }
    .fit-comfortable::before, .fit-tight::before, .fit-tooLarge::before { content: ''; display: inline-block; width: 8px; height: 8px; border-radius: 50%; margin-right: 4px; }
    .fit-comfortable::before { background: var(--ok); }
    .fit-tight::before { background: var(--lvl-warn); }
    .fit-tooLarge::before { background: var(--lvl-error); }
    .danger { color: var(--danger-text, var(--lvl-error)); }
    .err { color: var(--danger-text, var(--lvl-error)); }
  `,
})
export class AiModelManager implements OnInit {
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

  protected readonly others = computed(() => {
    const have = new Set(this.models().map((m) => m.fileName));
    return this.catalog().filter((r) => !have.has(r.model.file));
  });
  protected readonly engineState = computed(() => ENGINE_STATE[this.status.status()?.state ?? ''] ?? '');
  protected readonly size = formatBytes;
  protected readonly fit = fitLabel;

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
      this.progress.set(e.stage === 'ready' ? `${e.model} is now active.` : '');
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
    this.progress.set(`Loading ${m.name}…`);
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
      title: 'Delete model?',
      width: 460,
      inputs: {
        message: `Delete ${m.name}? The file (${formatBytes(m.sizeBytes)}) is removed from this computer${m.active ? ' and the assistant has no model until you choose another' : ''}.`,
        detail: m.fileName,
        confirmLabel: 'Delete',
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
    const path = await this.platform.pickOpenPath('Add a GGUF model', [{ name: 'GGUF model', extensions: ['gguf'] }]);
    if (!path) return;
    this.busy.set(true);
    this.error.set(null);
    try {
      const m = await this.backend.addLocalModel(path);
      this.progress.set(`${m.name} was added. Choose Use to make it active.`);
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
    this.dialogs.open(AiSetupWizard, { title: `Set up ${r.model.name}`, width: 760, inputs: { modelId: r.model.id } });
  }
}
