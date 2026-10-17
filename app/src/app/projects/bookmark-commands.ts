import { inject, Injectable } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { ImtCommand } from '../extensions/tokens';
import { BookmarkStore } from './bookmarks/bookmark.store';

/** Commands for bookmarks (the panel itself is the `bookmarks` view). */
@Injectable({ providedIn: 'root' })
export class BookmarkCommands {
  private readonly bookmarks = inject(BookmarkStore);
  private readonly workspace = inject(WorkspaceStore);

  readonly commands: ImtCommand[] = [
    { id: 'bookmarks.show', title: 'Show bookmarks', category: 'Analyze', run: (ctx) => ctx.showView('bookmarks') },
    {
      id: 'bookmarks.addSelected',
      title: 'Bookmark selected event',
      category: 'Analyze',
      keybinding: 'Mod+D',
      when: () => !!this.workspace.selected(),
      run: (ctx) => {
        this.bookmarks.addSelected();
        ctx.showView('bookmarks');
      },
    },
    {
      id: 'bookmarks.addRange',
      title: 'Bookmark time range',
      category: 'Analyze',
      when: () => this.workspace.hasRange(),
      run: (ctx) => {
        this.bookmarks.addRange();
        ctx.showView('bookmarks');
      },
    },
  ];
}
