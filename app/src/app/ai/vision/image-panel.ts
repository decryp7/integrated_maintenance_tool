import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { Platform } from '../../core/platform';
import { COMMAND_CONTEXT } from '../../extensions/command-registry';
import { JobsBackend } from '../../jobs/jobs-backend';
import { queuedLabel } from '../../jobs/jobs-text';
import { JobsStore } from '../../jobs/jobs.store';
import { AiStatusStore } from '../ai-status.store';
import { AiCitations } from '../chat/citation-actions';
import { ChatStore } from '../chat/chat.store';
import { AiRichText } from '../chat/rich-text';
import { AiImageStore } from './ai-image.store';
import { imageFilter } from './image-files';
import { ImageViewer } from './image-viewer';
import { AiNoVisionCard } from './no-vision-card';
import { AiRelatedResults } from './related-results';

/**
 * The image view (a dock view): an image opened for the AI with "Explain with AI", "Find related logs", "Ask in chat"
 * and "Close"; the explanation and the linked events below. A related-logs result opened from Jobs after its image
 * was closed is shown on its own.
 */
@Component({
  selector: 'debrief-ai-image-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ImageViewer, AiRichText, AiNoVisionCard, AiRelatedResults],
  template: `
    <div class="bar" role="toolbar" aria-label="Images" i18n-aria-label="@@ai.vision.panel.toolbar">
      @if (store.images().length > 1) {
        <label class="sr-only" for="ai-image-pick" i18n="@@ai.vision.panel.pick">Image</label>
        <select id="ai-image-pick" class="select" [value]="store.currentId() ?? ''" (change)="pick($event)" data-testid="ai-image-pick">
          @for (i of store.images(); track i.id) {
            <option [value]="i.id">{{ i.name }}</option>
          }
        </select>
      }
      <button type="button" class="btn ghost" [disabled]="store.opening()" (click)="openFile()" data-testid="ai-image-open" i18n="@@ai.vision.panel.open">Open image…</button>
    </div>

    @if (store.openError(); as e) {
      <p class="err" role="alert" data-testid="ai-image-open-error">
        {{ e }}
        <button type="button" class="link" (click)="store.retryOpen()" i18n="@@ai.vision.retry">Retry</button>
        <button type="button" class="link" (click)="store.dismissOpenError()" i18n="@@ai.vision.dismiss">Dismiss</button>
      </p>
    }
    @if (store.noVision()) {
      <debrief-ai-no-vision />
    }

    <div class="body">
      @if (store.current(); as img) {
        <debrief-image-viewer [image]="img">
          <span actions class="actions">
            <button type="button" class="btn primary small" [disabled]="!!explain()?.running" (click)="store.explain(img.id)" data-testid="ai-image-explain" i18n="@@ai.vision.panel.explain">Explain with AI</button>
            <button type="button" class="btn small" [disabled]="!!related()?.running" (click)="store.findRelated(img.id)" data-testid="ai-image-related" i18n="@@ai.vision.panel.related">Find related logs</button>
            <button type="button" class="btn ghost small" (click)="askInChat(img.id)" i18n="@@ai.vision.panel.ask">Ask in chat</button>
            <button type="button" class="btn ghost small" (click)="store.close(img.id)" [attr.aria-label]="closeLabel(img.name)" i18n="@@ai.vision.panel.close">Close</button>
          </span>
        </debrief-image-viewer>

        @if (explain(); as x) {
          <section class="result" aria-labelledby="ai-image-explain-title" data-testid="ai-image-explanation">
            <h4 id="ai-image-explain-title" i18n="@@ai.vision.panel.explanation">Explanation</h4>
            @if (x.running) {
              <p class="muted" role="status">{{ jobLine(x.jobId, explaining) }}
                @if (x.jobId !== null) {<button type="button" class="link" (click)="cancel(x.jobId)" i18n="@@ai.vision.cancel">Cancel</button>}
              </p>
            } @else if (x.error) {
              <p class="err" role="alert">{{ x.error }} <button type="button" class="link" (click)="store.explain(img.id)" i18n="@@ai.vision.retry">Retry</button></p>
            }
            @if (x.value; as n) {
              <debrief-ai-rich-text [text]="n.text" [citations]="n.citations" (record)="citations.record($event)" (query)="citations.query($event)" (link)="citations.web($event.url, $event.title)" />
            }
          </section>
        }

        @if (related(); as r) {
          <section class="result">
            @if (r.running) {
              <p class="muted" role="status" data-testid="ai-related-running">{{ jobLine(r.jobId, searching) }}
                @if (r.jobId !== null) {<button type="button" class="link" (click)="cancel(r.jobId)" i18n="@@ai.vision.cancel">Cancel</button>}
              </p>
            } @else if (r.error) {
              <p class="err" role="alert" data-testid="ai-related-error">{{ r.error }} <button type="button" class="link" (click)="store.findRelated(img.id)" i18n="@@ai.vision.retry">Retry</button></p>
            }
            @if (r.value; as o) {
              <debrief-ai-related-results [outcome]="o" />
            }
          </section>
        }
      } @else if (store.detached(); as d) {
        <p class="muted" data-testid="ai-image-detached">
          @if (d.imageName) {
            <ng-container i18n="@@ai.vision.panel.detachedNamed">{{ d.imageName }} is no longer open; its related logs are shown below.</ng-container>
          } @else {
            <ng-container i18n="@@ai.vision.panel.detached">The image is no longer open; its related logs are shown below.</ng-container>
          }
        </p>
        <debrief-ai-related-results [outcome]="d.outcome" />
      } @else {
        <div class="empty">
          <p class="muted" i18n="@@ai.vision.panel.empty">Open a screenshot to let the local AI explain it and find the log events that belong to it. Images stay on this computer.</p>
          @if (!status.status()?.vision && status.hasModel()) {
            <p class="muted small" i18n="@@ai.vision.panel.textOnly">The active model reads text only.</p>
          }
        </div>
      }
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; height: 100%; min-height: 0; font-size: var(--fs-md); }
    .bar { display: flex; align-items: center; gap: 6px; padding: 6px 8px; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
    .bar .select { max-width: 240px; }
    .body { flex: 1; overflow: auto; padding: 10px 12px; display: flex; flex-direction: column; gap: 12px; min-height: 0; }
    .actions { display: inline-flex; gap: 4px; flex-wrap: wrap; }
    .small { font-size: var(--fs-xs); }
    .result h4 { margin: 0 0 4px; font-size: var(--fs-md); }
    .result p { margin: 0 0 6px; }
    .err { color: var(--danger-text, var(--lvl-error)); margin: 0; padding: 4px 12px; }
    .result .err { padding: 0; }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; margin-left: 6px; }
    .link:focus-visible { outline: 2px solid var(--focus-ring); }
    .empty p { margin: 0 0 6px; }
    debrief-ai-no-vision { display: block; margin: 8px 12px 0; }
  `,
})
export class AiImagePanel {
  protected readonly store = inject(AiImageStore);
  protected readonly status = inject(AiStatusStore);
  protected readonly citations = inject(AiCitations);
  private readonly chat = inject(ChatStore);
  private readonly ctx = inject(COMMAND_CONTEXT);
  private readonly platform = inject(Platform);
  private readonly jobs = inject(JobsStore);
  private readonly jobsBackend = inject(JobsBackend);

  protected readonly explain = this.store.currentExplain;
  protected readonly related = this.store.currentRelated;
  protected readonly explaining = $localize`:@@ai.vision.panel.explaining:The local model reads the image…`;
  protected readonly searching = $localize`:@@ai.vision.panel.searching:The local model reads the image, then the engine searches the logs…`;
  private readonly jobsById = computed(() => new Map(this.jobs.snapshot().jobs.map((j) => [j.id, j])));

  /** Where a request is: queued, its progress message, or what runs. */
  protected jobLine(jobId: number | null, busy: string): string {
    const job = jobId === null ? undefined : this.jobsById().get(jobId);
    if (job?.state === 'queued') return queuedLabel(job.position);
    return job?.message ?? busy;
  }

  protected closeLabel(name: string): string {
    return $localize`:@@ai.vision.panel.closeLabel:Close ${name}:name:`;
  }

  protected pick(e: Event): void {
    this.store.select(Number((e.target as HTMLSelectElement).value));
  }

  protected async openFile(): Promise<void> {
    const path = await this.platform.pickOpenPath($localize`:@@ai.vision.pickTitle:Open an image`, imageFilter());
    if (path) await this.store.openPath(path);
  }

  protected async cancel(jobId: number): Promise<void> {
    await this.jobsBackend.cancel(jobId).catch(() => undefined);
  }

  /** Attaches the image to the chat's next question and shows the chat. */
  protected askInChat(id: number): void {
    const img = this.store.images().find((i) => i.id === id);
    if (!img) return;
    this.chat.attach(img);
    this.ctx.showView('ai-chat');
  }
}
