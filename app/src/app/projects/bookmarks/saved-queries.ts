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
      <input class="input grow" aria-label="Saved query name" placeholder="Name for the current query" [value]="name()" (input)="name.set(val($event))" (keydown.enter)="save()" />
      <button type="button" class="btn" (click)="save()">Save current query</button>
    </div>
    @if (queries.items().length) {
      <ul class="list" aria-label="Saved queries">
        @for (q of queries.items(); track q.id) {
          <li class="item">
            <span class="ellipsis grow" [title]="q.name">{{ q.name }}</span>
            <button type="button" class="btn" [attr.aria-label]="'Apply query ' + q.name" (click)="queries.apply(q.id)">Apply</button>
            <button type="button" class="icon-btn" [attr.aria-label]="'Delete query ' + q.name" title="Delete" (click)="queries.remove(q.id)"><imt-icon name="trash" [size]="13" /></button>
          </li>
        }
      </ul>
    } @else {
      <p class="faint">No saved queries.</p>
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

  protected save(): void {
    this.queries.saveCurrent(this.name());
    this.name.set('');
  }
  protected val = (e: Event) => (e.target as HTMLInputElement).value;
}
