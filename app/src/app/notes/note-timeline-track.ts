import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { formatDateTime } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { NoteActions } from './note-actions';
import { severityColor, TimeNote, timeNotes } from './notes-model';
import { NotesStore } from './notes.store';

/** A note placed on the visible range: its position and width (percent of the plot) and label row. */
export interface NoteMarkVm {
  note: TimeNote;
  left: number;
  /** Width of a range (percent); 0 for a moment. */
  width: number;
  row: number;
}

/** Places notes of the visible range (percent of the plot) and stacks labels that would overlap. Pure. */
export function layoutNoteMarks(notes: readonly TimeNote[], view: { from: number; to: number }, widthPx = 1000): NoteMarkVm[] {
  const span = Math.max(1, view.to - view.from);
  const rowEnds: number[] = [];
  return [...notes]
    .sort((a, b) => a.from - b.from || a.id.localeCompare(b.id))
    .map((note) => {
      const from = Math.max(note.from, view.from);
      const to = Math.min(note.to, view.to);
      const left = ((from - view.from) / span) * 100;
      const width = note.to > note.from ? ((to - from) / span) * 100 : 0;
      const x = (left / 100) * widthPx;
      let row = rowEnds.findIndex((end) => end <= x);
      if (row < 0) row = rowEnds.push(0) - 1;
      rowEnds[row] = x + Math.min(180, 20 + note.label.length * 6) + 4;
      return { note, left, width, row };
    });
}

/**
 * Notes on moments, ranges, markers and chart points as marks on the timeline (ADR 0021 §5): a flag with the note's
 * first line; a range also shades its span. Clicking a flag edits the note.
 */
@Component({
  selector: 'debrief-note-timeline-track',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @for (m of items(); track m.note.id) {
      @if (m.width > 0) {
        <div class="span" [style.left.%]="m.left" [style.width.%]="m.width" [style.--c]="color(m)" aria-hidden="true"></div>
      } @else {
        <div class="line" [style.left.%]="m.left" [style.--c]="color(m)" aria-hidden="true"></div>
      }
      <button
        type="button"
        class="flag"
        data-testid="note-mark"
        [style.left.%]="m.left"
        [style.top.px]="2 + m.row * 16"
        [style.--c]="color(m)"
        [title]="title(m)"
        [attr.aria-label]="title(m)"
        (pointerdown)="$event.stopPropagation()"
        (click)="open(m)"
      >✎ {{ m.note.label }}</button>
    }
  `,
  styles: `
    :host { position: absolute; inset: 0; pointer-events: none; }
    .line { position: absolute; top: 0; bottom: 0; width: 0; border-left: 2px dashed var(--c); opacity: .8; }
    .span { position: absolute; top: 0; bottom: 0; background: color-mix(in srgb, var(--c) 10%, transparent); border-left: 1px dashed var(--c); border-right: 1px dashed var(--c); }
    .flag { pointer-events: auto; position: absolute; max-width: 180px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font: inherit; font-size: var(--fs-2xs); line-height: 14px; padding: 0 5px; border-radius: 3px; border: 1px solid var(--c); background: var(--bg-3); color: var(--text); cursor: pointer; z-index: 2; }
    .flag:hover { background: var(--bg-hover); }
    .flag:focus-visible { outline: 2px solid var(--focus-ring); }
  `,
})
export class NoteTimelineTrack {
  /** The visible range (Unix ms). */
  readonly view = input<{ from: number; to: number } | null>(null);
  /** Plot width in px (for stacking labels). */
  readonly width = input(1000);
  private readonly store = inject(NotesStore);
  private readonly actions = inject(NoteActions);
  private readonly workspace = inject(WorkspaceStore);

  protected readonly items = computed(() => {
    const v = this.view();
    return v ? layoutNoteMarks(timeNotes(this.store.items(), v.from, v.to), v, this.width()) : [];
  });

  protected color(m: NoteMarkVm): string {
    return severityColor(m.note.severity);
  }

  protected title(m: NoteMarkVm): string {
    const when = formatDateTime(m.note.from, this.workspace.timezone(), true);
    const label = m.note.label;
    return $localize`:@@notes.timeline.title:Note at ${when}:when:: ${label}:label:. Click to edit.`;
  }

  protected open(m: NoteMarkVm): void {
    const n = this.store.get(m.note.id);
    if (n) void this.actions.edit(n);
  }
}
