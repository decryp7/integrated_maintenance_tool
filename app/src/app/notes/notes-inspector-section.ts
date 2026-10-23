import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { RecordView } from '../core/api.types';
import { MarkdownView } from '../share/report/markdown-view';
import { anchors, NoteActions } from './note-actions';
import { severityColor } from './notes-model';
import { NotesStore } from './notes.store';

/** The inspector's Notes section: the notes on the shown event, and "Add note". */
@Component({
  selector: 'debrief-notes-inspector-section',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkdownView],
  template: `
    @for (n of notes(); track n.id) {
      <div class="note" [style.--c]="color(n.severity)" data-testid="inspector-note">
        <debrief-markdown-view [text]="n.text" />
        <div class="actions">
          <button type="button" class="btn ghost small" (click)="actions.edit(n)" i18n="@@notes.inspector.edit">Edit</button>
          <button type="button" class="btn ghost small" (click)="actions.remove(n)" i18n="@@notes.inspector.delete">Delete</button>
        </div>
      </div>
    }
    <button type="button" class="btn small" (click)="add()" data-testid="inspector-add-note" i18n="@@notes.inspector.add">Add note…</button>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 6px; align-items: flex-start; }
    .note { align-self: stretch; border-left: 3px solid var(--c); padding-left: 8px; overflow-wrap: anywhere; }
    .actions { display: flex; gap: 4px; }
    .small { font-size: var(--fs-xs); }
  `,
})
export class NotesInspectorSection {
  readonly record = input.required<RecordView>();
  protected readonly actions = inject(NoteActions);
  private readonly store = inject(NotesStore);
  protected readonly notes = computed(() => this.store.byEvent().get(this.record().id) ?? []);
  protected readonly color = severityColor;

  protected add(): void {
    void this.actions.write(anchors.event(this.record()));
  }
}
