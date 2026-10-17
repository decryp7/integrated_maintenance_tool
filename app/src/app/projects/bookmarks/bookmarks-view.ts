import { ChangeDetectionStrategy, Component } from '@angular/core';
import { BookmarkList } from './bookmark-list';
import { HighlightEditor } from './highlight-editor';
import { SavedQueries } from './saved-queries';

/** The "Bookmarks" panel: bookmarks with notes, saved queries and highlight rules. All are saved with the project. */
@Component({
  selector: 'imt-bookmarks-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [BookmarkList, SavedQueries, HighlightEditor],
  template: `
    <div class="scroll" role="region" aria-label="Bookmarks" tabindex="0">
      <section aria-labelledby="bm-h"><h2 id="bm-h" class="section-title">Bookmarks and notes</h2><imt-bookmark-list /></section>
      <section aria-labelledby="sq-h"><h2 id="sq-h" class="section-title">Saved queries</h2><imt-saved-queries /></section>
      <section aria-labelledby="hl-h"><h2 id="hl-h" class="section-title">Highlight rules</h2><imt-highlight-editor /></section>
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 18px; max-width: 820px; }
    .scroll:focus-visible { outline-offset: -2px; }
    section { display: flex; flex-direction: column; gap: 8px; }
    h2 { margin: 0; }
  `,
})
export class BookmarksView {}
