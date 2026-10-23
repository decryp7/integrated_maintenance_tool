import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { ParserLibraryDto } from '../../core/api.types';
import { ParserEditor } from './parser-editor';
import { ParsersSettings } from './parsers-settings';
import { ParsersStore } from './parsers.store';

/**
 * The Parsers document (I3): the library on the left and, beside it, the selected parser's TOML with its validation
 * and a live preview against the open files, so the data a parser is about stays in view. Opened from Tools and
 * "Manage parsers…"; Settings ▸ Parsers links here.
 */
@Component({
  selector: 'debrief-parsers-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  host: { class: 'parsers-view' },
  imports: [ParsersSettings, ParserEditor],
  template: `
    <div class="list" role="region" aria-label="Parser library" i18n-aria-label="@@parsers.view.library" tabindex="0">
      <debrief-parsers-settings [inline]="true" />
    </div>
    <div class="detail" role="region" aria-label="Parser detail" i18n-aria-label="@@parsers.view.detail">
      @for (e of editing(); track e.doc.id + ':' + e.isNew) {
        <debrief-parser-editor [doc]="e.doc" [isNew]="e.isNew" [readSources]="e.readSources" (closed)="done($event)" />
      } @empty {
        <p class="hint" data-testid="parsers-detail-hint" i18n="@@parsers.view.hint">Choose Edit on a parser to see its TOML here, checked by the engine as you type and previewed against your open files.</p>
      }
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: grid; grid-template-columns: minmax(320px, 2fr) minmax(360px, 3fr); background: var(--bg-1); }
    .list { position: relative; min-width: 0; overflow: auto; padding: 12px 14px; border-inline-end: 1px solid var(--border); }
    .detail { min-width: 0; min-height: 0; overflow: auto; padding: 12px 14px; display: flex; flex-direction: column; }
    .detail debrief-parser-editor { flex: 1; min-height: 0; }
    .hint { margin: 0; color: var(--muted); line-height: 1.5; }
    @media (max-width: 900px) { :host { grid-template-columns: 1fr; grid-template-rows: auto minmax(0, 1fr); } .list { border-inline-end: 0; border-bottom: 1px solid var(--border); max-height: 45%; } }
  `,
})
export class ParsersView {
  private readonly store = inject(ParsersStore);
  protected readonly editing = () => {
    const e = this.store.editing();
    return e ? [e] : [];
  };

  protected done(library: ParserLibraryDto | undefined): void {
    this.store.closeEditor(library);
  }
}
