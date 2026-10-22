import { ChangeDetectionStrategy, Component, computed, inject, linkedSignal, untracked } from '@angular/core';
import { formatDateTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { SettingsStore } from '../settings/settings.store';
import { FrameImage } from './frame-image';
import { VideoError } from './video-error';
import { frameAlignedMs, frameName } from './video-format';
import { VIDEO_STYLES } from './video-styles';
import { VideoStore } from './video.store';

/** The longest note or description the engine keeps (ADR 0033). */
const MAX_TEXT = 2000;

/**
 * The selected frame: the picture, when it was (in the video and on the logs' clock), the model's description (read-only
 * text), the user's note and what can be done with the frame: Explain with AI, Find related logs, Describe, Remove.
 */
@Component({
  selector: 'debrief-frame-detail',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FrameImage, VideoError],
  template: `
    @if (store.errors().frame; as e) {
      <debrief-video-error [failure]="e" (retry)="store.retry('frame')" (dismiss)="store.dismiss('frame')" />
    }
    @if (selected(); as sel) {
      @if (store.active(); as v) {
        <div class="detail" data-testid="video-frame-detail">
          <debrief-frame-image class="big" [videoKey]="v.record.key" [index]="sel.index" [image]="sel.frame.image" [alt]="name()" [eager]="true" />
          <div class="side">
            <h3 data-testid="video-frame-name">{{ name() }}</h3>
            <p class="muted" data-testid="video-frame-aligned" i18n="@@video.frame.aligned">On the logs' clock: {{ aligned() }}</p>
            <p class="muted"><ng-container i18n="@@video.frame.description">Description</ng-container>:
              @if (sel.frame.description; as d) {
                <span class="desc" data-testid="video-frame-description">{{ d }}</span>
              } @else {
                <span i18n="@@video.frame.noDescription">none yet</span>
              }
            </p>
            <label class="sr" for="video-note" i18n="@@video.frame.noteLabel">Note on this frame</label>
            <textarea id="video-note" data-testid="video-note" [value]="draft()" [maxLength]="max" (input)="draft.set(textOf($event))" placeholder="Add a note on this frame" i18n-placeholder="@@video.frame.notePlaceholder"></textarea>
            <div class="row">
              <button class="btn primary" type="button" data-testid="video-note-save" [disabled]="!dirty() || store.busy() !== null" (click)="saveNote(sel.index)" i18n="@@video.frame.saveNote">Save note</button>
              <button class="btn" type="button" data-testid="video-frame-explain" [disabled]="store.busy() !== null" (click)="store.useWithAi('explain', sel.index)" i18n="@@video.frame.explain">Explain with AI</button>
              <button class="btn" type="button" data-testid="video-frame-related" [disabled]="store.busy() !== null" (click)="store.useWithAi('related', sel.index)" i18n="@@video.frame.related">Find related logs</button>
              <button class="btn" type="button" data-testid="video-frame-describe" [disabled]="store.busy() !== null" (click)="store.describe([sel.index], !!sel.frame.description)">
                {{ sel.frame.description ? describeAgain : describeLabel }}
              </button>
              <button class="btn" type="button" data-testid="video-frame-remove" [disabled]="store.busy() !== null || v.record.frames.length <= 1" (click)="store.removeFrame(sel.index)" i18n="@@video.frame.remove">Remove frame</button>
            </div>
          </div>
        </div>
      }
    } @else {
      <p class="muted" data-testid="video-no-frame" i18n="@@video.frame.none">Choose a frame to see it larger and to add a note.</p>
    }
  `,
  styles: [
    VIDEO_STYLES,
    `
      .detail { display: grid; grid-template-columns: minmax(200px, 1fr) minmax(220px, 1fr); gap: 12px; align-items: start; }
      .big { border-radius: 4px; overflow: hidden; min-height: 120px; }
      .desc { color: var(--text); }
      @media (max-width: 720px) { .detail { grid-template-columns: 1fr; } }
    `,
  ],
})
export class FrameDetail {
  protected readonly store = inject(VideoStore);
  private readonly workspace = inject(WorkspaceStore);
  private readonly settings = inject(SettingsStore);
  protected readonly max = MAX_TEXT;
  protected readonly describeLabel = $localize`:@@video.frame.describe:Describe`;
  protected readonly describeAgain = $localize`:@@video.frame.describeAgain:Describe again`;

  protected readonly selected = this.store.frame;
  protected readonly name = computed(() => {
    const s = this.selected();
    return s ? frameName(s.index, s.frame) : '';
  });
  protected readonly aligned = computed(() => {
    const v = this.store.active();
    const s = this.selected();
    return v && s ? formatDateTime(frameAlignedMs(v.record, s.frame), this.workspace.timezone(), true, this.settings.dateFormat()) : '';
  });

  /** The note being edited: the saved note whenever the frame (or its saved note) changes, not on unrelated updates. */
  protected readonly draft = linkedSignal<string, string>({
    source: computed(() => {
      const s = this.selected();
      return s ? `${this.store.active()?.record.key}/${s.index}/${s.frame.note ?? ''}` : '';
    }),
    computation: () => untracked(() => this.selected()?.frame.note ?? ''),
  });
  protected readonly dirty = computed(() => this.draft() !== (this.selected()?.frame.note ?? ''));

  protected textOf(e: Event): string {
    return (e.target as HTMLTextAreaElement).value;
  }

  protected async saveNote(index: number): Promise<void> {
    await this.store.saveNote(index, this.draft());
  }
}
