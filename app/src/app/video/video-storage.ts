import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { formatBytes } from '../shared/format/units';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { DialogService } from '../ui/dialog.service';
import { VideoError } from './video-error';
import { removalOf, removalText, selectionNoteText, selectionText } from './video-format';
import { VIDEO_STYLES } from './video-styles';
import { VideoStore } from './video.store';

/** The most frames one video keeps (ADR 0033). */
const MAX_VIDEO_FRAMES = 120;

/**
 * What the video costs the project: its frames against the project's cap, the optional copy of the video in the project
 * (with its size), and removing the video (after asking: it deletes the frames, notes and descriptions). The way the
 * frames were chosen, and what the selection had to do about it, is explained here too.
 */
@Component({
  selector: 'debrief-video-storage',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [VideoError],
  template: `
    <h3 i18n="@@video.storage.title">Storage</h3>
    @if (store.errors().storage; as e) {
      <debrief-video-error [failure]="e" (retry)="store.retry('storage')" (dismiss)="store.dismiss('storage')" />
    }
    @if (store.active(); as v) {
      <p data-testid="video-frames-count" i18n="@@video.storage.frames">This video keeps {{ v.record.frames.length }} of {{ maxVideoFrames }} frames; the project keeps {{ store.projectFrames() }} of {{ store.maxProjectFrames() }}.</p>
      <div class="meter" [class.full]="store.projectFull()" role="progressbar" aria-label="Frames in the project" i18n-aria-label="@@video.storage.meterLabel" aria-valuemin="0" [attr.aria-valuemax]="store.maxProjectFrames()" [attr.aria-valuenow]="store.projectFrames()">
        <span [style.width.%]="percent()"></span>
      </div>
      @if (store.projectFull()) {
        <p class="warn" role="status" data-testid="video-project-full" i18n="@@video.storage.full">The project is full: remove frames or a video before adding more.</p>
      }
      <div class="row">
        <label class="field">
          <input type="checkbox" role="switch" data-testid="video-include" [checked]="v.record.included" [disabled]="store.busy() !== null || (v.media === 'missing' && !v.record.included)" (change)="toggleInclude($event)" />
          <span i18n="@@video.storage.include">Include the video in the project ({{ size() }})</span>
        </label>
      </div>
      <p class="muted" i18n="@@video.storage.includeHint">Without a copy the project remembers where the video is; on another computer you can locate it again.</p>
      <div class="row">
        <button class="btn danger" type="button" data-testid="video-remove" [disabled]="store.busy() !== null" (click)="confirmRemove()" i18n="@@video.storage.remove">Remove video…</button>
      </div>
      <h3 i18n="@@video.select.title">How the frames were chosen</h3>
      <p data-testid="video-selection">{{ selection() }}</p>
      @for (n of notes(); track $index) {
        <p class="muted" data-testid="video-selection-note">{{ n }}</p>
      }
    }
  `,
  styles: [VIDEO_STYLES, `.warn { color: var(--warn-text); } .btn.danger { color: var(--danger-text); border-color: var(--danger-text); }`],
})
export class VideoStorage {
  protected readonly store = inject(VideoStore);
  private readonly dialogs = inject(DialogService);
  protected readonly maxVideoFrames = MAX_VIDEO_FRAMES;

  protected readonly size = computed(() => formatBytes(this.store.active()?.record.size ?? 0));
  protected readonly percent = computed(() => Math.min(100, (this.store.projectFrames() / Math.max(1, this.store.maxProjectFrames())) * 100));
  protected readonly selection = computed(() => {
    const o = this.store.active()?.record.options;
    return o ? selectionText(o) : '';
  });
  protected readonly notes = computed(() => (this.store.active()?.record.selectionNotes ?? []).map(selectionNoteText));

  /** Including copies the whole video into the project: asks first, with its size. */
  protected async toggleInclude(e: Event): Promise<void> {
    const box = e.target as HTMLInputElement;
    const v = this.store.active();
    if (!v) return;
    if (!box.checked) {
      await this.store.include(false);
      return;
    }
    // Not yet: the box follows the engine's answer.
    box.checked = false;
    const size = this.size();
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:Dialog title@@video.storage.include.title:Include the video?`,
      width: 480,
      inputs: {
        message: $localize`:@@video.storage.include.message:A copy of ${v.record.name}:name: (${size}:size:) is stored in the project and goes with it when the project is shared.`,
        confirmLabel: $localize`:@@video.storage.include.confirm:Include`,
      },
    });
    if ((await ref.closed) === true) {
      await this.store.include(true);
      box.checked = this.store.active()?.record.included ?? false;
    }
  }

  protected async confirmRemove(): Promise<void> {
    const v = this.store.active();
    if (!v) return;
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:Dialog title@@video.storage.remove.title:Remove the video?`,
      width: 520,
      inputs: { message: removalText(v.record.name, removalOf(v.record)), confirmLabel: $localize`:@@video.storage.remove.confirm:Remove video`, danger: true },
    });
    if ((await ref.closed) === true) await this.store.remove(v.record.key);
  }
}
