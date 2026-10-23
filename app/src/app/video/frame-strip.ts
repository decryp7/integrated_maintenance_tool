import { CdkListbox, CdkOption } from '@angular/cdk/listbox';
import { ChangeDetectionStrategy, Component, computed, inject, input, output, signal } from '@angular/core';
import { FrameImage } from './frame-image';
import { formatVideoTime, frameName, parseOffsetSeconds } from './video-format';
import { VIDEO_STYLES } from './video-styles';
import { VideoStore } from './video.store';

/**
 * The kept frames of the video as a strip: picture, time in the video, a mark when it has a description or a note. A
 * keyboard-navigable listbox (arrows, Home/End); choosing a frame selects it for the panels beside. Also adds a frame
 * at the player's position and describes the frames that have no description.
 */
@Component({
  selector: 'debrief-frame-strip',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CdkListbox, CdkOption, FrameImage],
  template: `
    @if (store.active(); as v) {
      <div class="row head">
        <h3 class="grow" i18n="@@video.strip.title">Frames</h3>
        <label class="field">
          <span i18n="@@video.strip.at">At (seconds)</span>
          <input type="text" inputmode="decimal" data-testid="video-add-at" [value]="shownAt()" (input)="atText.set(valueOf($event))" aria-label="Time in the video, in seconds" i18n-aria-label="@@video.strip.atLabel" />
        </label>
        <button class="btn" type="button" data-testid="video-add-frame" [disabled]="store.busy() !== null || atMs() === null" (click)="add()" i18n="@@video.strip.add">Add frame at player position</button>
        <button class="btn" type="button" data-testid="video-describe-all" [disabled]="store.busy() !== null || !undescribed()" (click)="store.describe([])" i18n="@@video.strip.describeAll">Describe frames without a description</button>
      </div>
      @if (store.busy() === 'describe') {
        <p class="muted" role="status" data-testid="video-describing" i18n="@@video.strip.describing">The model is describing the frames. Progress and Cancel are in Jobs.</p>
      }
      <ul cdkListbox cdkListboxOrientation="horizontal" class="strip" data-testid="video-strip" aria-label="Frames of the video" i18n-aria-label="@@video.strip.label" [cdkListboxValue]="selected()" (cdkListboxValueChange)="choose($event.value[0])">
        @for (f of v.record.frames; track f.image + f.videoMs; let i = $index) {
          <li [cdkOption]="i" class="frame" [attr.data-testid]="'video-frame-' + i" [attr.aria-label]="optionLabel(i, f.videoMs, !!f.description, !!f.note)">
            <debrief-frame-image class="thumb" [videoKey]="v.record.key" [index]="i" [image]="f.image" alt="" />
            <span class="time mono">{{ time(f.videoMs) }}</span>
            <span class="marks">
              @if (f.description) { <span class="mark" title="Described" i18n-title="@@video.strip.described">D</span> }
              @if (f.note) { <span class="mark note" title="Has a note" i18n-title="@@video.strip.hasNote">N</span> }
              @if (f.manual) { <span class="mark" title="Added by you" i18n-title="@@video.strip.manual">+</span> }
            </span>
          </li>
        }
      </ul>
    }
  `,
  styles: [
    VIDEO_STYLES,
    `
      .head h3 { margin: 0; }
      .strip { list-style: none; margin: 0; padding: 4px 0 8px; display: flex; gap: 8px; overflow-x: auto; }
      .frame { flex: none; width: 128px; display: flex; flex-direction: column; gap: 2px; padding: 3px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); cursor: pointer; }
      .frame[aria-selected='true'] { border-color: var(--accent); background: var(--bg-sel); }
      .frame:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
      .thumb { height: 72px; border-radius: 3px; overflow: hidden; }
      .time { font-size: var(--fs-2xs); color: var(--text-2); }
      .marks { display: flex; gap: 3px; min-height: 14px; }
      .mark { font-family: var(--font-mono); font-size: var(--fs-2xs); padding: 0 3px; border: 1px solid var(--border-3); border-radius: 3px; color: var(--muted); }
      .mark.note { color: var(--chip-text); border-color: var(--chip-border); }
    `,
  ],
})
export class FrameStrip {
  protected readonly store = inject(VideoStore);
  /** The player's position (ms), offered as the time of "Add frame". */
  readonly playerMs = input(0);
  readonly chosen = output<number>();

  protected readonly atText = signal<string | null>(null);
  protected readonly selected = computed(() => {
    const i = this.store.selectedIndex();
    return i === null ? [] : [i];
  });
  protected readonly undescribed = computed(() => (this.store.active()?.record.frames ?? []).some((f) => !f.description));
  protected readonly shownAt = computed(() => this.atText() ?? String(Math.round(this.playerMs()) / 1000));
  /** The typed time, else the player's. */
  protected readonly atMs = computed(() => {
    const typed = this.atText();
    return typed === null ? this.playerMs() : parseOffsetSeconds(typed);
  });

  protected valueOf(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }

  protected time(ms: number): string {
    return formatVideoTime(ms);
  }

  protected optionLabel(index: number, videoMs: number, described: boolean, note: boolean): string {
    const base = frameName(index, { videoMs });
    const extra = [described ? $localize`:@@video.strip.optDescribed:described` : '', note ? $localize`:@@video.strip.optNote:has a note` : ''].filter(Boolean);
    return extra.length ? `${base}, ${extra.join(', ')}` : base;
  }

  protected choose(value: unknown): void {
    if (typeof value === 'number') {
      this.store.selectFrame(value);
      this.chosen.emit(value);
    }
  }

  protected async add(): Promise<void> {
    const ms = this.atMs();
    if (ms === null) return;
    await this.store.addFrame(ms);
    this.atText.set(null);
  }
}
