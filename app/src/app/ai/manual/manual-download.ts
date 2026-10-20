import { Clipboard } from '@angular/cdk/clipboard';
import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  output,
  signal,
  untracked,
} from '@angular/core';
import { Platform } from '../../core/platform';
import { AiBackend, toAiError } from '../ai-backend';
import { formatBytes } from '../ai-format';
import { AiErrorCode, InstalledModel, ManualDownload, ManualFile, ManualKind } from '../ai.types';
import { AiRuntimeStore } from '../runtimes/ai-runtime.store';
import { failureHeadline, hostOf } from './manual-download-view';

let nextId = 0;

/**
 * "Download manually": when IMT cannot download a model or an inference runtime itself (a TLS-inspecting
 * proxy such as Zscaler, a blocked host), the user downloads the exact files in the browser and hands them
 * back; the engine checks their SHA-256 before using them. Shows the failure kind, the files with their
 * expected SHA-256, step-by-step instructions and the button that adds the downloaded files.
 */
@Component({
  selector: 'imt-ai-manual-download',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <section class="manual" [attr.aria-labelledby]="titleId" data-testid="ai-manual-download">
      <h4 [id]="titleId" i18n="@@ai.manual.title">Download manually</h4>
      @if (headline()) {
        <p class="kind" data-testid="ai-manual-kind">{{ headline() }}</p>
      }
      @if (loadError(); as err) {
        <p class="err" role="alert">
          {{ err }}
          <button type="button" class="btn ghost" (click)="load()" i18n="@@ai.manual.retry">Retry</button>
        </p>
      }
      @if (info(); as d) {
        <ul class="files" aria-label="Files to download" i18n-aria-label="@@ai.manual.files.label">
          @for (f of d.files; track f.name; let i = $index) {
            <li class="file" [attr.data-testid]="'ai-manual-file-' + i">
              <div class="line">
                <span class="mono name">{{ f.name }}</span>
                <span class="muted">{{ size(f.sizeBytes) }}</span>
                <button
                  type="button"
                  class="btn primary"
                  data-testid="ai-manual-open"
                  [attr.aria-label]="openLabel(f)"
                  (click)="open(i)"
                  i18n="@@ai.manual.open"
                >
                  Download manually
                </button>
              </div>
              @if (f.sha256; as sha) {
                <div class="line sha">
                  <span i18n="@@ai.manual.sha">Expected SHA-256:</span>
                  <code class="mono" data-testid="ai-manual-sha">{{ sha }}</code>
                  <button
                    type="button"
                    class="btn ghost"
                    [attr.aria-label]="copyShaLabel(f)"
                    (click)="copy(sha)"
                    i18n="@@ai.manual.copy"
                  >
                    Copy
                  </button>
                </div>
              } @else {
                <p class="muted" data-testid="ai-manual-sha-unknown" i18n="@@ai.manual.shaUnknown">
                  The SHA-256 of this file is not known, so IMT cannot check it.
                </p>
              }
            </li>
          }
        </ul>
        <h5 i18n="@@ai.manual.steps.title">How to install it by hand</h5>
        <ol class="steps" data-testid="ai-manual-steps">
          @if (d.kind === 'model') {
            <li i18n="@@ai.manual.model.step1">Choose <strong>Download manually</strong>: your browser downloads {{ d.files[0]?.name }} from {{ host() }}. Wait until it has finished; the file is large.</li>
            <li i18n="@@ai.manual.model.step2">Choose <strong>Add downloaded file…</strong> below (or <strong>Add local GGUF…</strong> in the model manager) and pick the file.</li>
            <li i18n="@@ai.manual.model.step3">IMT checks its SHA-256 against the value above, files it in the models folder and lists it under Installed models; then choose Retry or Use.</li>
          } @else {
            <li i18n="@@ai.manual.runtime.step1">Choose <strong>Download manually</strong> for each file above: your browser downloads it from {{ host() }}.</li>
            <li i18n="@@ai.manual.runtime.step2">Choose <strong>Add downloaded files…</strong> below and pick all of them (do not unzip them).</li>
            <li i18n="@@ai.manual.runtime.step3">IMT checks their SHA-256 against the values above and unpacks them into the folder below.</li>
          }
        </ol>
        <div class="line folder">
          <span i18n="@@ai.manual.folder">Folder:</span>
          <code class="mono" data-testid="ai-manual-folder">{{ d.folder }}</code>
          <button
            type="button"
            class="btn ghost"
            aria-label="Copy the folder path"
            i18n-aria-label="@@ai.manual.copyFolder.label"
            (click)="copy(d.folder)"
            i18n="@@ai.manual.copy"
          >
            Copy
          </button>
        </div>
        @if (d.kind === 'runtime') {
          <p class="muted" data-testid="ai-manual-custom" i18n="@@ai.manual.runtime.custom">
            If your IT department provides llama-server, set <strong>Custom llama-server</strong> in Settings → AI to
            its path instead; the downloaded builds are then not used.
          </p>
        }
        <div class="actions">
          <button
            type="button"
            class="btn"
            data-testid="ai-manual-add"
            [disabled]="busy()"
            (click)="add(d)"
          >
            {{ addLabel(d.kind) }}
          </button>
        </div>
      }
      <div class="sr-only" aria-live="polite">{{ status() }}</div>
      @if (status()) {
        <p class="muted" role="status" data-testid="ai-manual-status">{{ status() }}</p>
      }
      @if (error(); as err) {
        <p class="err" role="alert" data-testid="ai-manual-error">{{ err }}</p>
      }
    </section>
  `,
  styles: `
    :host {
      display: block;
    }
    .manual {
      display: flex;
      flex-direction: column;
      gap: 6px;
      padding: 8px 10px;
      border: 1px solid var(--border);
      border-radius: 6px;
      background: var(--bg-1);
    }
    h4,
    h5 {
      margin: 0;
      font-size: var(--fs-sm);
    }
    p {
      margin: 0;
    }
    .kind {
      font-weight: 600;
    }
    .files,
    .steps {
      margin: 0;
      padding-left: 18px;
      display: flex;
      flex-direction: column;
      gap: 4px;
    }
    .files {
      list-style: none;
      padding-left: 0;
    }
    .line {
      display: flex;
      flex-wrap: wrap;
      gap: 6px;
      align-items: center;
    }
    .name,
    code {
      overflow-wrap: anywhere;
    }
    .sha,
    .folder {
      font-size: var(--fs-sm);
      color: var(--text-2);
    }
    .actions {
      display: flex;
      gap: 6px;
    }
    .muted {
      color: var(--muted);
      font-size: var(--fs-sm);
    }
    .err {
      color: var(--danger-text, var(--lvl-error));
    }
  `,
})
export class AiManualDownload {
  private readonly backend = inject(AiBackend);
  private readonly platform = inject(Platform);
  private readonly clipboard = inject(Clipboard);
  private readonly runtimes = inject(AiRuntimeStore);

  readonly kind = input.required<ManualKind>();
  /** The catalog model id or the runtime build id; the engine computes every address from it. */
  readonly id = input.required<string>();
  /** Why the download failed, for the headline. */
  readonly code = input<AiErrorCode | null>(null);
  /** A model file was checked and added (models), or the install from the archives started (runtimes). */
  readonly added = output<InstalledModel | null>();

  protected readonly titleId = `ai-manual-title-${nextId++}`;
  protected readonly info = signal<ManualDownload | null>(null);
  protected readonly loadError = signal<string | null>(null);
  protected readonly busy = signal(false);
  protected readonly status = signal('');
  protected readonly error = signal<string | null>(null);
  protected readonly headline = computed(() => failureHeadline(this.code()));
  protected readonly host = computed(() => hostOf(this.info()?.files[0]?.url ?? ''));
  protected readonly size = formatBytes;

  constructor() {
    effect(() => {
      const kind = this.kind();
      const id = this.id();
      untracked(() => void this.load(kind, id));
    });
  }

  async load(kind = this.kind(), id = this.id()): Promise<void> {
    this.loadError.set(null);
    try {
      this.info.set(await this.backend.manualDownload(kind, id));
    } catch (e) {
      this.info.set(null);
      this.loadError.set(toAiError(e).message);
    }
  }

  protected openLabel(f: ManualFile): string {
    return $localize`:@@ai.manual.open.label:Download manually: ${f.name}:name:`;
  }

  protected copyShaLabel(f: ManualFile): string {
    return $localize`:@@ai.manual.copySha.label:Copy the SHA-256 of ${f.name}:name:`;
  }

  protected addLabel(kind: ManualKind): string {
    return kind === 'model'
      ? $localize`:@@ai.manual.addModel:Add downloaded file…`
      : $localize`:@@ai.manual.addRuntime:Add downloaded files…`;
  }

  async open(index: number): Promise<void> {
    this.error.set(null);
    try {
      await this.backend.openManualDownload(this.kind(), this.id(), index);
      this.status.set($localize`:@@ai.manual.opened:The download was opened in your browser.`);
    } catch (e) {
      this.error.set(toAiError(e).message);
    }
  }

  protected copy(text: string): void {
    this.status.set(
      this.clipboard.copy(text)
        ? $localize`:@@ai.manual.copied:Copied.`
        : $localize`:@@ai.manual.copyFailed:The clipboard is not available.`,
    );
  }

  async add(d: ManualDownload): Promise<void> {
    this.error.set(null);
    if (d.kind === 'model') {
      const path = await this.platform.pickOpenPath($localize`:@@ai.manual.pickModel:Add the downloaded model`, [
        { name: $localize`:@@ai.models.addLocal.filter:GGUF model`, extensions: ['gguf'] },
      ]);
      if (!path) return;
      this.busy.set(true);
      this.status.set($localize`:@@ai.manual.checking:Checking the SHA-256 of the file… (cancel it in Jobs)`);
      try {
        const m = await this.backend.addLocalModel(path);
        this.status.set($localize`:@@ai.manual.added:${m.name}:name: was checked and added.`);
        this.added.emit(m);
      } catch (e) {
        this.status.set('');
        this.error.set(toAiError(e).message);
      } finally {
        this.busy.set(false);
      }
      return;
    }
    const paths = await this.platform.pickOpenPaths($localize`:@@ai.manual.pickRuntime:Add the downloaded archives`, [
      { name: $localize`:@@ai.manual.archives:Archives`, extensions: ['zip', 'gz'] },
    ]);
    if (!paths.length) return;
    // The runtime panel shows the check and unpacking (progress, outcome) like a download.
    this.status.set('');
    await this.runtimes.installFromArchives(d.id, paths);
    this.added.emit(null);
  }
}
