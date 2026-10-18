import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { Icon } from '../../shared/icon';
import { SavedQueryStore } from './saved-query.store';

/** Named queries: save the current search and filters, re-apply them with one click. */
@Component({
  selector: 'imt-saved-queries',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <div class="bar">
      <input class="input grow" aria-label="Saved query name" i18n-aria-label="@@projects.savedQueries.name" placeholder="Name for the current query" i18n-placeholder="@@projects.savedQueries.namePlaceholder" [value]="name()" (input)="name.set(val($event))" (keydown.enter)="save()" />
      <button type="button" class="btn" (click)="save()" i18n="@@projects.savedQueries.save">Save current query</button>
    </div>
    @if (queries.items().length) {
      <ul class="list" aria-label="Saved queries" i18n-aria-label="@@projects.savedQueries.list">
        @for (q of queries.items(); track q.id) {
          <li class="item">
            <span class="ellipsis grow" [title]="q.name">{{ q.name }}</span>
            <button type="button" class="btn" [attr.aria-label]="applyLabel(q.name)" (click)="queries.apply(q.id)" i18n="@@projects.savedQueries.apply">Apply</button>
            <button type="button" class="icon-btn" [attr.aria-label]="deleteLabel(q.name)" title="Delete" i18n-title="@@projects.savedQueries.delete" (click)="queries.remove(q.id)"><imt-icon name="trash" [size]="13" /></button>
          </li>
        }
      </ul>
    } @else {
      <p class="faint" i18n="@@projects.savedQueries.empty">No saved queries.</p>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; }
    .bar { display: flex; gap: 8px; }
    .grow { flex: 1; min-width: 0; }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    .item { display: flex; align-items: center; gap: 8px; padding: 4px 8px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); }
    p { margin: 0; }
  `,
})
export class SavedQueries {
  protected readonly queries = inject(SavedQueryStore);
  protected readonly name = signal('');

  protected applyLabel(name: string): string {
    return $localize`:@@projects.savedQueries.applyAria:Apply query ${name}:name:`;
  }
  protected deleteLabel(name: string): string {
    return $localize`:@@projects.savedQueries.deleteAria:Delete query ${name}:name:`;
  }

  protected save(): void {
    this.queries.saveCurrent(this.name());
    this.name.set('');
  }
  protected val = (e: Event) => (e.target as HTMLInputElement).value;
}
