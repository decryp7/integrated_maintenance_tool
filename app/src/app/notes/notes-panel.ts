import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { NoteItem } from '../core/api.types';
import { formatFull } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { hasEventDrag, readEventDrag } from '../core/event-drag';
import { MarkdownView } from '../share/report/markdown-view';
import { anchors, NoteActions } from './note-actions';
import { anchorSpan, anchorText, kindLabel, NOTE_KINDS, NoteKind, severityColor, severityLabel } from './notes-model';
import { NotesPersistence } from './notes-persistence';
import { NotesStore } from './notes.store';

/**
 * The Notes panel (a dock view, ADR 0021 §5): every note of the project with what it is about, filtered by kind, tag
 * and text; each jumps to its anchor, edits, or deletes after confirming. "Add note" writes one on the project.
 */
@Component({
  selector: 'debrief-notes-panel',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkdownView],
  host: {
    '[class.drop-target]': 'dropping()',
    '(dragover)': 'onDragOver($event)',
    '(dragleave)': 'dropping.set(false)',
    '(drop)': 'onDrop($event)',
  },
  template: `
    <div class="bar" role="toolbar" aria-label="Notes" i18n-aria-label="@@notes.panel.toolbar">
      <input class="input grow" type="search" [value]="store.filter().text" (input)="store.setFilter({ text: value($event) })" placeholder="Filter notes" i18n-placeholder="@@notes.panel.filter" aria-label="Filter notes" i18n-aria-label="@@notes.panel.filterLabel" data-testid="notes-filter" />
      <select class="select" [value]="store.filter().kind ?? ''" (change)="kind($event)" aria-label="Kind" i18n-aria-label="@@notes.panel.kind" data-testid="notes-kind">
        <option value="" i18n="@@notes.panel.allKinds">All kinds</option>
        @for (k of kinds; track k) {
          <option [value]="k">{{ kindName(k) }}</option>
        }
      </select>
      @if (store.tags().length) {
        <select class="select" [value]="store.filter().tag ?? ''" (change)="tag($event)" aria-label="Tag" i18n-aria-label="@@notes.panel.tag" data-testid="notes-tag">
          <option value="" i18n="@@notes.panel.allTags">All tags</option>
          @for (t of store.tags(); track t) {
            <option [value]="t">#{{ t }}</option>
          }
        </select>
      }
      <button type="button" class="btn" (click)="actions.write(projectAnchor)" data-testid="notes-add-project" i18n="@@notes.panel.addProject">Add note</button>
    </div>
    @if (persistence.warning(); as w) {
      <p class="warn" role="status">{{ w }}</p>
    }
    @if (store.loadError(); as e) {
      <p class="err" role="alert">{{ e }} <button type="button" class="link" (click)="store.load()" i18n="@@notes.panel.retry">Retry</button></p>
    }
    @if (store.actionError(); as e) {
      <p class="err" role="alert">{{ e }} <button type="button" class="link" (click)="store.dismissError()" i18n="@@notes.panel.dismiss">Dismiss</button></p>
    }
    <div class="scroll">
      @if (store.filtered().length) {
        <ul class="list" aria-label="Notes" i18n-aria-label="@@notes.panel.list">
          @for (n of store.filtered(); track n.id) {
            <li class="item" [style.--c]="color(n)" data-testid="note-item">
              <div class="top">
                <span class="badge">{{ kindName(n.anchor.kind) }}</span>
                <span class="sev" [title]="severityName(n)">{{ severityName(n) }}</span>
                <span class="about ellipsis" [title]="about(n)">{{ about(n) }}</span>
                @if (!n.resolved) {
                  <span class="gone" title="What this note is about is no longer in the workspace" i18n-title="@@notes.panel.goneTitle" i18n="@@notes.panel.gone">missing</span>
                }
              </div>
              @if (when(n); as w) {
                <div class="when faint">{{ w }}</div>
              }
              <debrief-markdown-view class="text" [text]="n.text" />
              @if (n.tags.length) {
                <div class="tags">
                  @for (t of n.tags; track t) {
                    <button type="button" class="tag" (click)="store.setFilter({ tag: t })">#{{ t }}</button>
                  }
                </div>
              }
              <div class="actions">
                <span class="faint meta">{{ meta(n) }}</span>
                <button type="button" class="btn ghost small" [disabled]="n.anchor.kind === 'project'" (click)="actions.jump(n)" data-testid="note-jump" i18n="@@notes.panel.jump">Jump to</button>
                <button type="button" class="btn ghost small" (click)="actions.edit(n)" data-testid="note-edit" i18n="@@notes.panel.edit">Edit</button>
                <button type="button" class="btn ghost small" (click)="actions.remove(n)" data-testid="note-delete" i18n="@@notes.panel.delete">Delete</button>
              </div>
            </li>
          }
        </ul>
      } @else if (store.items().length) {
        <p class="faint" i18n="@@notes.panel.noMatch">No note matches the filter.</p>
        <button type="button" class="btn ghost" (click)="store.clearFilter()" i18n="@@notes.panel.clearFilter">Clear the filter</button>
      } @else if (!store.loading()) {
        <p class="faint" data-testid="notes-empty" i18n="@@notes.panel.empty">No notes yet. Right-click an event, a source or the timeline's time axis, or mark an area of an image, to write down what you see.</p>
      }
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); font-size: var(--fs-md); }
    :host(.drop-target) { outline: 2px dashed var(--accent); outline-offset: -3px; }
    .bar { display: flex; gap: 6px; padding: 6px 8px; border-bottom: 1px solid var(--border); flex-wrap: wrap; align-items: center; }
    .grow { flex: 1; min-width: 120px; }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 8px; }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
    .item { border: 1px solid var(--border-2); border-left: 3px solid var(--c); border-radius: 6px; background: var(--bg-2); padding: 6px 8px; display: flex; flex-direction: column; gap: 4px; }
    .top { display: flex; align-items: center; gap: 6px; min-width: 0; }
    .badge { font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: .04em; color: var(--muted); }
    .sev { font-size: var(--fs-xs); color: var(--c); font-weight: 600; }
    .about { flex: 1; min-width: 0; }
    .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .gone { font-size: var(--fs-xs); color: var(--danger-text); border: 1px solid currentColor; border-radius: 8px; padding: 0 6px; }
    .when, .meta { font-size: var(--fs-xs); }
    .text { overflow-wrap: anywhere; }
    .tags { display: flex; flex-wrap: wrap; gap: 4px; }
    .tag { all: unset; cursor: pointer; font-size: var(--fs-xs); color: var(--accent-text); }
    .tag:focus-visible { outline: 2px solid var(--focus-ring); }
    .actions { display: flex; align-items: center; gap: 4px; }
    .meta { flex: 1; }
    .small { font-size: var(--fs-xs); }
    .warn { margin: 0; padding: 4px 12px; color: var(--text-2); }
    .err { margin: 0; padding: 4px 12px; color: var(--danger-text); }
    .link { all: unset; cursor: pointer; color: var(--accent-text); text-decoration: underline; margin-left: 6px; }
    p { margin: 0 0 8px; }
  `,
})
export class NotesPanel {
  protected readonly store = inject(NotesStore);
  protected readonly actions = inject(NoteActions);
  protected readonly persistence = inject(NotesPersistence);
  private readonly workspace = inject(WorkspaceStore);
  protected readonly kinds = NOTE_KINDS;
  protected readonly projectAnchor = anchors.project();
  /** An event is being dragged over the panel. */
  protected readonly dropping = signal(false);
  /** Accepts a dragged event (from the events grid). */
  protected onDragOver(e: DragEvent): void {
    if (!hasEventDrag(e.dataTransfer)) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
    this.dropping.set(true);
  }

  /** Dropping an event writes a note on it. */
  protected onDrop(e: DragEvent): void {
    this.dropping.set(false);
    const event = readEventDrag(e.dataTransfer);
    if (!event) return;
    e.preventDefault();
    void this.actions.write(anchors.event({ id: event.id, ts: event.ts, sourceName: event.source, message: event.excerpt }));
  }

  private readonly tz = computed(() => this.workspace.timezone());

  constructor() {
    void this.store.load();
  }

  protected kindName(k: NoteKind): string {
    return kindLabel(k);
  }
  protected severityName(n: NoteItem): string {
    return severityLabel(n.severity);
  }
  protected color(n: NoteItem): string {
    return severityColor(n.severity);
  }
  protected about(n: NoteItem): string {
    return anchorText(n.anchor);
  }
  protected when(n: NoteItem): string | null {
    const span = anchorSpan(n.anchor);
    if (!span) return null;
    const [a, b] = span;
    return a === b ? formatFull(a, this.tz()) : `${formatFull(a, this.tz())} → ${formatFull(b, this.tz())}`;
  }
  protected meta(n: NoteItem): string {
    const at = formatFull(n.updatedMs, this.tz());
    return n.author ? `${n.author} · ${at}` : at;
  }
  protected value(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }
  protected kind(e: Event): void {
    const v = (e.target as HTMLSelectElement).value;
    this.store.setFilter({ kind: (NOTE_KINDS as readonly string[]).includes(v) ? (v as NoteKind) : null });
  }
  protected tag(e: Event): void {
    const v = (e.target as HTMLSelectElement).value;
    this.store.setFilter({ tag: v || null });
  }
}
