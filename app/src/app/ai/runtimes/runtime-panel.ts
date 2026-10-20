import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  OnInit,
  signal,
  untracked,
} from '@angular/core';
import { ConfirmDialog } from '../../ui/confirm-dialog';
import { DialogService } from '../../ui/dialog.service';
import { formatBytes } from '../ai-format';
import { AiStatusStore } from '../ai-status.store';
import { AiErrorCode, AiInstalledRuntime, AiRuntimeEntry } from '../ai.types';
import { AiManualDownload } from '../manual/manual-download';
import { offersManualDownload } from '../manual/manual-download-view';
import { AiRuntimeStore } from './ai-runtime.store';
import {
  deviceLine,
  entryState,
  entryStateLabel,
  runtimePercent,
  runtimeProgressText,
  runtimeStageLabel,
} from './runtime-view';

/**
 * The inference runtimes: the llama-server builds for this computer with their size, whether they are
 * installed and work here, which one is suggested and which one runs the model; download (with progress
 * and cancel), delete, "Use this" and automatic choice. Used by the set-up wizard and the model manager.
 */
@Component({
  selector: 'imt-ai-runtime-panel',
  imports: [AiManualDownload],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="intro muted">
      <ng-container i18n="@@ai.runtime.intro.start">The model runs in llama.cpp's <span class="mono">llama-server</span>, downloaded once from github.com/ggml-org/llama.cpp </ng-container>
      @if (store.overview(); as o) {
        <ng-container i18n="@@ai.runtime.intro.release">(release {{ o.release }}) </ng-container>
      }
      <ng-container i18n="@@ai.runtime.intro.end">and checked against its pinned checksum.</ng-container>
    </div>
    <div class="sr-only" aria-live="polite">{{ liveText() }}</div>
    @if (store.overview()?.customPath; as path) {
      <p class="note" role="note" data-testid="ai-runtime-custom" i18n="@@ai.runtime.custom">
        A custom llama-server is set in Settings → AI (<span class="mono">{{ path }}</span
        >); the downloaded builds are not used.
      </p>
    }
    @if (store.overview()?.problem; as problem) {
      <p class="note warn" role="note" data-testid="ai-runtime-problem" i18n="@@ai.runtime.problem">
        The builds could not be checked: {{ problem }}
      </p>
    }
    @if (store.error(); as err) {
      <p class="err" role="alert" data-testid="ai-runtime-load-error">
        {{ err }}
        <button type="button" class="btn ghost" (click)="store.load()" i18n="@@ai.runtime.retry">Retry</button>
      </p>
    }
    @if (store.job(); as i) {
      <div class="progress" data-testid="ai-runtime-progress">
        <p role="status">{{ stageText() }}</p>
        @if (pct() !== null) {
          <div
            class="bar"
            role="progressbar"
            aria-label="Runtime download progress"
            i18n-aria-label="@@ai.runtime.progress.label"
            aria-valuemin="0"
            aria-valuemax="100"
            [attr.aria-valuenow]="pct()"
          >
            <span class="fill" [style.width.%]="pct()"></span>
          </div>
          <p class="muted">{{ progressText() }}</p>
        }
        <div class="row-actions">
          <button
            type="button"
            class="btn"
            data-testid="ai-runtime-cancel"
            [disabled]="i.cancelling"
            (click)="store.cancel()"
          >
            {{ cancelLabel(i.cancelling) }}
          </button>
        </div>
      </div>
    } @else if (store.suggested(); as s) {
      <div class="callout" data-testid="ai-runtime-suggested">
        <p>
          <ng-container i18n="@@ai.runtime.suggested.text"><strong>{{ s.label }}</strong> is recommended for this computer ({{ size(s.downloadBytes) }} download). </ng-container>
          @if (store.needsDownload()) {
            <ng-container i18n="@@ai.runtime.suggested.needs">The assistant needs an inference runtime before it can load a model.</ng-container>
          }
        </p>
        <button
          type="button"
          class="btn primary"
          data-testid="ai-runtime-download-suggested"
          (click)="store.install(s.id)"
          i18n="@@ai.runtime.suggested.download"
        >
          Download {{ s.label }}
        </button>
      </div>
    }
    @if (store.outcome(); as out) {
      @switch (out.stage) {
        @case ('failed') {
          <p class="err" role="alert" data-testid="ai-runtime-error">
            <ng-container i18n="@@ai.runtime.failed">{{ labelOf(out.id) }} was not installed: {{ out.message }} </ng-container>
            <button type="button" class="btn ghost" (click)="store.install(out.id)" i18n="@@ai.runtime.tryAgain">
              Try again
            </button>
          </p>
        }
        @case ('cancelled') {
          <p class="muted" role="status" data-testid="ai-runtime-cancelled" i18n="@@ai.runtime.cancelled">
            The download of {{ labelOf(out.id) }} was cancelled. It resumes where it stopped next
            time.
          </p>
        }
        @case ('ready') {
          <p class="ok" role="status" data-testid="ai-runtime-ready" i18n="@@ai.runtime.installed">
            {{ labelOf(out.id) }} is installed.
          </p>
        }
      }
    }
    @if (manual(); as m) {
      <imt-ai-manual-download kind="runtime" [id]="m.id" [code]="m.code" />
    }
    <div class="choice" data-testid="ai-runtime-choice">
      @if (store.overview(); as o) {
        @if (o.choice === 'auto') {
          <span
            ><ng-container i18n="@@ai.runtime.choice.auto">Automatic choice</ng-container
            ><span class="muted">{{ autoTail(o.active) }}</span></span
          >
        } @else {
          <span
            ><ng-container i18n="@@ai.runtime.choice.fixed">Fixed to {{ labelOf(o.choice) }}</ng-container
            ><span class="muted" i18n="@@ai.runtime.choice.fallback"> (falls back to the next build if it cannot start).</span></span
          >
          <button
            type="button"
            class="btn ghost"
            data-testid="ai-runtime-auto"
            [disabled]="busy()"
            (click)="store.choose('auto')"
            i18n="@@ai.runtime.choice.useAuto"
          >
            Use automatic
          </button>
        }
      }
    </div>
    @if (store.loading() && !store.overview()) {
      <p class="muted" role="status" i18n="@@ai.runtime.checking">Checking the inference runtimes…</p>
    }
    <ul class="list" aria-label="Inference runtimes" i18n-aria-label="@@ai.runtime.list.label">
      @for (b of store.builds(); track b.id) {
        <li class="row" [class.active]="b.active" [attr.data-testid]="'ai-runtime-' + b.id">
          <div class="main">
            <span class="name">
              {{ b.label }}
              @if (b.suggested) {
                <span class="badge suggested" i18n="@@ai.runtime.badge.suggested">Suggested</span>
              }
              @if (b.active) {
                <span class="badge active" i18n="runtime build in use|@@ai.runtime.badge.active">Active</span>
              }
              <span class="badge" [class]="'badge state-' + state(b)">{{ stateLabel(b) }}</span>
            </span>
            <span class="facts">
              @if (b.installed) {
                <ng-container i18n="@@ai.runtime.onDisk">{{ size(b.installedBytes ?? 0) }} on disk</ng-container>
              } @else {
                <ng-container i18n="@@ai.runtime.downloadSize">{{ size(b.downloadBytes) }} download</ng-container>
              }
            </span>
            @for (d of b.devices; track d.name) {
              <span class="facts device">{{ device(d) }}</span>
            }
            @if (b.problem) {
              <span class="err">{{ b.problem }}</span>
            }
          </div>
          <div class="actions">
            @if (!b.installed) {
              <button
                type="button"
                class="btn"
                [disabled]="busy() || store.installing()"
                (click)="store.install(b.id)"
                [attr.aria-label]="downloadLabel(b)"
              >
                <ng-container i18n="@@ai.runtime.download">Download</ng-container>
              </button>
            } @else {
              @if (store.overview()?.choice !== b.id) {
                <button
                  type="button"
                  class="btn"
                  [disabled]="busy() || b.usable === false"
                  (click)="store.choose(b.id)"
                  [attr.aria-label]="useLabel(b)"
                >
                  <ng-container i18n="@@ai.runtime.useThis">Use this</ng-container>
                </button>
              }
              <button
                type="button"
                class="btn ghost danger"
                [disabled]="busy()"
                (click)="remove(b)"
                [attr.aria-label]="deleteLabel(b)"
              >
                <ng-container i18n="@@ai.runtime.delete">Delete</ng-container>
              </button>
            }
          </div>
        </li>
      }
    </ul>
    @if (store.overview()?.outdated?.length) {
      <h4 i18n="@@ai.runtime.older.title">Older releases on disk</h4>
      <ul class="list" aria-label="Older inference runtimes" i18n-aria-label="@@ai.runtime.older.label">
        @for (r of store.overview()!.outdated; track r.tag + '/' + r.id) {
          <li class="row" [attr.data-testid]="'ai-runtime-outdated-' + r.tag + '-' + r.id">
            <div class="main">
              <span class="name"
                >{{ labelOf(r.id) }} <span class="badge">{{ r.tag }}</span></span
              >
              <span class="facts" i18n="@@ai.runtime.older.facts">{{ size(r.bytes) }} on disk · no longer offered</span>
            </div>
            <div class="actions">
              <button
                type="button"
                class="btn ghost danger"
                [disabled]="busy()"
                (click)="removeOutdated(r)"
                [attr.aria-label]="deleteOutdatedLabel(r)"
              >
                <ng-container i18n="@@ai.runtime.delete">Delete</ng-container>
              </button>
            </div>
          </li>
        }
      </ul>
    }
  `,
  styles: `
    :host {
      display: flex;
      flex-direction: column;
      gap: 8px;
    }
    p {
      margin: 0;
    }
    h4 {
      margin: 4px 0 0;
      font-size: var(--fs-sm);
    }
    .intro {
      font-size: var(--fs-sm);
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
    .row.active {
      border-color: var(--accent);
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
    .actions,
    .row-actions {
      display: flex;
      gap: 6px;
    }
    .badge.suggested,
    .badge.active {
      font-weight: 600;
    }
    .state-active::before,
    .state-ready::before,
    .state-problem::before {
      content: '';
      display: inline-block;
      width: 8px;
      height: 8px;
      border-radius: 50%;
      margin-right: 4px;
    }
    .state-active::before,
    .state-ready::before {
      background: var(--ok);
    }
    .state-problem::before {
      background: var(--lvl-error);
    }
    .callout {
      display: flex;
      gap: 10px;
      align-items: center;
      justify-content: space-between;
      padding: 8px 10px;
      border: 1px solid var(--accent);
      border-radius: 6px;
      background: var(--bg-1);
    }
    .choice {
      display: flex;
      gap: 8px;
      align-items: center;
      font-size: var(--fs-sm);
    }
    .progress {
      display: flex;
      flex-direction: column;
      gap: 4px;
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
    .note {
      color: var(--muted);
      font-size: var(--fs-sm);
    }
    .note.warn {
      color: var(--lvl-warn);
    }
    .ok {
      color: var(--ok);
    }
    .danger {
      color: var(--danger-text, var(--lvl-error));
    }
    .err {
      color: var(--danger-text, var(--lvl-error));
    }
  `,
})
export class AiRuntimePanel implements OnInit {
  protected readonly store = inject(AiRuntimeStore);
  private readonly status = inject(AiStatusStore);
  private readonly dialogs = inject(DialogService);

  protected readonly busy = computed(() => this.store.busy());
  /** The build whose download failed in a way a manual download works around; kept while the user tries
   * the files they downloaded (a rejected archive keeps the instructions), cleared once it is installed. */
  protected readonly manual = signal<{ id: string; code: AiErrorCode } | null>(null);
  protected readonly pct = computed(() => runtimePercent(this.store.job()?.stage ?? null));
  protected readonly progressText = computed(() =>
    runtimeProgressText(this.store.job()?.stage ?? null),
  );
  protected readonly stageText = computed(() => {
    const i = this.store.job();
    if (!i) return '';
    return i.stage
      ? runtimeStageLabel(i.stage, this.labelOf(i.id))
      : $localize`:@@ai.runtime.starting:Starting the download of ${this.labelOf(i.id)}:label:…`;
  });
  /** Announced to screen readers when an install starts or ends. */
  protected readonly liveText = computed(() => {
    const out = this.store.outcome();
    if (this.store.job()) return $localize`:@@ai.runtime.live.downloading:Downloading ${this.labelOf(this.store.job()!.id)}:label:.`;
    if (!out) return '';
    return out.stage === 'ready'
      ? $localize`:@@ai.runtime.live.installed:${this.labelOf(out.id)}:label: is installed.`
      : out.stage === 'cancelled'
        ? $localize`:@@ai.runtime.live.cancelled:The download was cancelled.`
        : $localize`:@@ai.runtime.live.failed:The download failed: ${out.message}:message:`;
  });

  protected readonly size = formatBytes;
  protected readonly state = entryState;
  protected readonly stateLabel = entryStateLabel;
  protected readonly device = deviceLine;

  constructor() {
    effect(() => {
      const out = this.store.outcome();
      untracked(() => {
        if (out?.stage === 'failed' && offersManualDownload(out.code)) this.manual.set({ id: out.id, code: out.code });
        else if (out?.stage === 'ready' && this.manual()?.id === out.id) this.manual.set(null);
      });
    });
  }

  ngOnInit(): void {
    void this.store.load();
  }

  protected cancelLabel(cancelling: boolean): string {
    return cancelling ? $localize`:@@ai.runtime.cancelling:Cancelling…` : $localize`:@@ai.runtime.cancel:Cancel download`;
  }

  protected autoTail(active: string | null | undefined): string {
    return active
      ? $localize`:@@ai.runtime.choice.autoActive: — the best installed build that works here: ${this.labelOf(active)}:label:.`
      : $localize`:@@ai.runtime.choice.autoNone: — the best installed build that works here.`;
  }

  protected downloadLabel(b: AiRuntimeEntry): string {
    return $localize`:@@ai.runtime.download.label:Download ${b.label}:label:`;
  }

  protected useLabel(b: AiRuntimeEntry): string {
    return $localize`:@@ai.runtime.useThis.label:Use this: ${b.label}:label:`;
  }

  protected deleteLabel(b: AiRuntimeEntry): string {
    return $localize`:@@ai.runtime.delete.label:Delete ${b.label}:label:`;
  }

  protected deleteOutdatedLabel(r: AiInstalledRuntime): string {
    return $localize`:@@ai.runtime.deleteOutdated.label:Delete ${this.labelOf(r.id)}:label: ${r.tag}:tag:`;
  }

  protected labelOf(id: string): string {
    return this.store.overview()?.builds.find((b) => b.id === id)?.label ?? id;
  }

  protected async remove(b: AiRuntimeEntry): Promise<void> {
    const tag = this.store.overview()?.tag;
    if (!tag) return;
    const size = formatBytes(b.installedBytes ?? 0);
    const ok = await this.confirm(
      b.active
        ? $localize`:@@ai.runtime.confirm.deleteActive:Delete the ${b.label}:label: runtime? Its files (${size}:size:) are removed from this computer; the engine stops and uses the next working build, or asks for a download.`
        : $localize`:@@ai.runtime.confirm.delete:Delete the ${b.label}:label: runtime? Its files (${size}:size:) are removed from this computer.`,
    );
    if (ok && (await this.store.remove(tag, b.id))) void this.status.refresh();
  }

  protected async removeOutdated(r: AiInstalledRuntime): Promise<void> {
    if (
      await this.confirm(
        $localize`:@@ai.runtime.confirm.deleteOutdated:Delete the ${this.labelOf(r.id)}:label: runtime of release ${r.tag}:tag: (${formatBytes(r.bytes)}:size:)?`,
      )
    )
      await this.store.remove(r.tag, r.id);
  }

  private async confirm(message: string): Promise<boolean> {
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:@@ai.runtime.confirm.title:Delete inference runtime?`,
      width: 460,
      inputs: { message, confirmLabel: $localize`:@@ai.runtime.delete:Delete`, danger: true },

    });
    return (await ref.closed) === true;
  }
}
