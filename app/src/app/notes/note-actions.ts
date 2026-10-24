import { inject, Injectable } from '@angular/core';
import { NoteAnchor, NoteItem, RecordView } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { ConfirmDialog } from '../ui/confirm-dialog';
import { DialogService } from '../ui/dialog.service';
import { NoteImageStore } from './image/note-image.store';
import { anchorText, firstLine } from './notes-model';
import { NotesStore } from './notes.store';

/** The Notes panel's view id. */
export const NOTES_VIEW = 'notes';
/** The image view's id (an image of the workspace with its marked regions). */
export const NOTE_IMAGE_VIEW = 'note-image';

/** Anchors for what the views offer notes on (the engine fills and checks them). */
export const anchors = {
  /** An event, from the grid's row or from a dragged event. */
  event: (r: Pick<RecordView, 'id' | 'ts' | 'sourceName' | 'message'>): NoteAnchor => ({ kind: 'event', eventId: r.id, ts: r.ts, source: r.sourceName, excerpt: r.message.split('\n')[0] ?? '' }),
  instant: (at: number): NoteAnchor => ({ kind: 'instant', at: Math.round(at) }),
  range: (from: number, to: number): NoteAnchor => ({ kind: 'range', from: Math.round(Math.min(from, to)), to: Math.round(Math.max(from, to)) }),
  source: (path: string): NoteAnchor => ({ kind: 'source', path }),
  /** A timeline marker (`to` null) or span (a run, an annotation band). */
  marker: (name: string, from: number, to: number | null = null): NoteAnchor => ({
    kind: 'marker',
    name,
    from: Math.round(from),
    to: to === null || to <= from ? null : Math.round(to),
  }),
  /** A point of a chart (`series` and `value` null: a moment on the chart's plot). */
  chartPoint: (chart: string, series: string | null, at: number, value: number | null): NoteAnchor => ({ kind: 'chartPoint', chart, series, at: Math.round(at), value }),
  project: (): NoteAnchor => ({ kind: 'project' }),
};

/** Most events a jump to a moment shows around it (± this many ms). */
const MOMENT_MS = 30_000;

/**
 * What every place that offers notes does: write one (the editor dialog), edit, delete after confirming, and jump to
 * a note's anchor (the event in the grid, the time range, the image with its region).
 */
@Injectable({ providedIn: 'root' })
export class NoteActions {
  private readonly dialogs = inject(DialogService);
  private readonly store = inject(NotesStore);
  private readonly workspace = inject(WorkspaceStore);
  private readonly images = inject(NoteImageStore);
  private readonly ctx = inject(COMMAND_CONTEXT);

  /** Opens the editor for a new note on `anchor`; resolves with the new note's id (null when cancelled). */
  async write(anchor: NoteAnchor): Promise<string | null> {
    const ref = this.dialogs.openLazy(() => import('./note-editor').then((m) => m.NoteEditor), { title: $localize`:@@notes.actions.newTitle:Add a note`, width: 620, inputs: { anchor } });
    return (await ref.closed) ?? null;
  }

  async edit(note: NoteItem): Promise<string | null> {
    const ref = this.dialogs.openLazy(() => import('./note-editor').then((m) => m.NoteEditor), { title: $localize`:@@notes.actions.editTitle:Edit note`, width: 620, inputs: { note } });
    return (await ref.closed) ?? null;
  }

  /** Deletes `note` after the user confirms; true when it was deleted. */
  async remove(note: NoteItem): Promise<boolean> {
    const label = firstLine(note.text, 60) || anchorText(note.anchor);
    const ref = this.dialogs.open<boolean>(ConfirmDialog, {
      title: $localize`:@@notes.actions.deleteTitle:Delete note`,
      width: 460,
      inputs: {
        message: $localize`:@@notes.actions.deleteMessage:Delete the note "${label}:label:"? This cannot be undone.`,
        confirmLabel: $localize`:@@notes.actions.deleteConfirm:Delete`,
        danger: true,
      },
    });
    if ((await ref.closed) !== true) return false;
    return (await this.store.remove(note.id)) === true;
  }

  /** Writes a note on the event shown in the inspector. False when nothing is selected. */
  async writeOnSelected(): Promise<boolean> {
    const r = this.workspace.selected();
    if (!r) return false;
    await this.write(anchors.event(r));
    return true;
  }

  /** Writes a note on the current query's time range. False when no range is set. */
  async writeOnRange(): Promise<boolean> {
    const { from, to } = this.workspace.query();
    if (from == null || to == null || from >= to) return false;
    await this.write(anchors.range(from, to));
    return true;
  }

  /** Shows what the note is about. */
  async jump(note: NoteItem): Promise<void> {
    const a = note.anchor;
    switch (a.kind) {
      case 'event':
        this.ctx.showView('events');
        this.workspace.jumpTo(a.eventId);
        return;
      case 'instant':
      case 'chartPoint':
        this.workspace.setRange(a.at - MOMENT_MS, a.at + MOMENT_MS);
        this.ctx.showView('events');
        return;
      case 'range':
        this.workspace.setRange(a.from, a.to);
        this.ctx.showView('events');
        return;
      case 'marker':
        this.workspace.setRange(a.from - MOMENT_MS, (a.to ?? a.from) + MOMENT_MS);
        this.ctx.showView('events');
        return;
      case 'source': {
        const src = this.workspace.summary()?.sources.find((s) => s.path === a.path);
        if (src) this.workspace.patchQuery({ sources: [src.id] });
        this.ctx.showView('events');
        return;
      }
      case 'image':
        this.ctx.showView(NOTE_IMAGE_VIEW);
        await this.images.showKey(a.image.key, note.id);
        return;
      case 'project':
        this.ctx.showView(NOTES_VIEW);
        return;
    }
  }
}
