import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { Bookmark } from '../../core/tools.types';
import { formatFull } from '../../core/format';
import { WorkspaceStore } from '../../core/workspace.store';
import { Icon } from '../../shared/icon';
import { BookmarkStore } from './bookmark.store';

/** Bookmarks on records and time ranges, each with an editable note and a jump action. */
@Component({
  selector: 'imt-bookmark-list',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <div class="bar">
      <button type="button" class="btn" [disabled]="!workspace.selected()" (click)="bookmarks.addSelected()" i18n="@@projects.bookmarkList.addSelected">Bookmark selected event</button>
      <button type="button" class="btn" [disabled]="!workspace.hasRange()" (click)="bookmarks.addRange()" i18n="@@projects.bookmarkList.addRange">Bookmark time range</button>
    </div>
    @if (bookmarks.items().length) {
      <ul class="list" aria-label="Bookmarks" i18n-aria-label="@@projects.bookmarkList.list">
        @for (b of bookmarks.items(); track b.id) {
          <li class="item">
            <div class="top">
              <span class="badge">@if (b.kind === 'record') { <ng-container i18n="@@projects.bookmarkList.kindEvent">event</ng-container> } @else { <ng-container i18n="@@projects.bookmarkList.kindRange">range</ng-container> }</span>
              <span class="label ellipsis" [title]="b.label">{{ b.label }}</span>
              <span class="faint when">{{ when(b) }}</span>
              <button type="button" class="btn" [attr.aria-label]="jumpLabel(b.label)" (click)="bookmarks.jump(b)" i18n="@@projects.bookmarkList.jump">Jump to</button>
              <button type="button" class="icon-btn" [attr.aria-label]="deleteLabel(b.label)" title="Delete" i18n-title="@@projects.bookmarkList.delete" (click)="bookmarks.remove(b.id)"><imt-icon name="trash" [size]="13" /></button>
            </div>
            <input class="input note" [attr.aria-label]="noteLabel(b.label)" placeholder="Add a note…" i18n-placeholder="@@projects.bookmarkList.notePlaceholder" [value]="b.note" (change)="bookmarks.setNote(b.id, val($event))" />
          </li>
        }
      </ul>
    } @else {
      <p class="faint" i18n="@@projects.bookmarkList.empty">No bookmarks yet. Select an event or a time range, then bookmark it to come back later.</p>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; }
    .bar { display: flex; gap: 8px; flex-wrap: wrap; }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    .item { padding: 6px 8px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); display: flex; flex-direction: column; gap: 6px; }
    .top { display: flex; align-items: center; gap: 8px; }
    .label { flex: 1; min-width: 0; }
    .when { font-size: var(--fs-xs); white-space: nowrap; }
    .note { width: 100%; box-sizing: border-box; }
    p { margin: 0; }
  `,
})
export class BookmarkList {
  protected readonly bookmarks = inject(BookmarkStore);
  protected readonly workspace = inject(WorkspaceStore);

  protected jumpLabel(label: string): string {
    return $localize`:@@projects.bookmarkList.jumpAria:Jump to bookmark ${label}:label:`;
  }
  protected deleteLabel(label: string): string {
    return $localize`:@@projects.bookmarkList.deleteAria:Delete bookmark ${label}:label:`;
  }
  protected noteLabel(label: string): string {
    return $localize`:@@projects.bookmarkList.noteAria:Note for ${label}:label:`;
  }

  protected when(b: Bookmark): string {
    const tz = this.workspace.timezone();
    if (b.kind === 'range') return `${formatFull(b.from ?? null, tz)} → ${formatFull(b.to ?? null, tz)}`;
    return formatFull(b.ts ?? null, tz);
  }
  protected val = (e: Event) => (e.target as HTMLInputElement).value;
}
