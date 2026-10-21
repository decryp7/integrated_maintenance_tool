import { ChangeDetectionStrategy, Component, computed, inject, OnInit } from '@angular/core';
import { formatBytes } from '../ai-format';
import { AiLauncher } from '../ai-launcher';
import { JobsStore } from '../../jobs/jobs.store';
import { AiImageStore } from './ai-image.store';

/**
 * The active model reads text only: offers its image support (the catalog projector, downloaded only when the user
 * asks) or another model marked "Text + images". Shared by the chat and the image view.
 */
@Component({
  selector: 'imt-ai-no-vision',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="card" role="note" data-testid="ai-no-vision">
      <p class="msg">
        @if (store.activeModel(); as m) {
          <ng-container i18n="@@ai.vision.noVision.named">{{ m.name }} reads text only, so it cannot look at images.</ng-container>
        } @else {
          <ng-container i18n="@@ai.vision.noVision.message">The active model reads text only, so it cannot look at images.</ng-container>
        }
      </p>
      <div class="actions">
        @if (store.projector().running) {
          <span class="muted" role="status" data-testid="ai-projector-running">{{ runningText() }}</span>
        } @else if (downloadBytes(); as bytes) {
          <button type="button" class="btn primary" data-testid="ai-add-image-support" (click)="store.installProjector()" i18n="@@ai.vision.noVision.add">Add image support ({{ size(bytes) }})</button>
        }
        <button type="button" class="btn" data-testid="ai-choose-vision-model" (click)="launcher.openModels()" i18n="@@ai.vision.noVision.choose">Choose a model that reads images…</button>
        <button type="button" class="btn ghost" (click)="store.dismissNoVision()" i18n="@@ai.vision.noVision.dismiss">Dismiss</button>
      </div>
      @if (store.projector().error; as e) {
        <p class="err" role="alert">{{ e }} <button type="button" class="link" (click)="store.installProjector()" i18n="@@ai.vision.retry">Retry</button></p>
      }
    </div>
  `,
  styles: `
    .card { border: 1px solid var(--border); border-left: 3px solid var(--lvl-warn); border-radius: 6px; padding: 8px 10px; background: var(--bg-1); display: flex; flex-direction: column; gap: 6px; }
    .msg { margin: 0; }
    .actions { display: flex; gap: 6px; flex-wrap: wrap; align-items: center; }
    .err { margin: 0; color: var(--danger-text, var(--lvl-error)); }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); }
  `,
})
export class AiNoVisionCard implements OnInit {
  protected readonly store = inject(AiImageStore);
  protected readonly launcher = inject(AiLauncher);
  private readonly jobs = inject(JobsStore);
  protected readonly size = formatBytes;
  protected readonly downloadBytes = computed(() => this.store.activeModel()?.projectorDownloadBytes ?? null);
  /** The download's progress as the Jobs pane shows it. */
  protected readonly runningText = computed(() => {
    const id = this.store.projector().jobId;
    const job = id === null ? null : this.jobs.snapshot().jobs.find((j) => j.id === id);
    const f = job?.progress.fraction;
    return f != null && f > 0
      ? $localize`:@@ai.vision.noVision.progress:Downloading image support… ${Math.round(f * 100)}:percent: % (see Jobs)`
      : $localize`:@@ai.vision.noVision.running:Downloading image support… (see Jobs)`;
  });

  ngOnInit(): void {
    void this.store.loadActiveModel();
  }
}
