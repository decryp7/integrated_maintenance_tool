import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { FilterSetStore } from './filter-set.store';
import { LensStore } from './lens.store';

/** Number keys that toggle the first nine filter sets together with Alt (`event.code`, so the keyboard layout does not matter). */
export const SET_KEY_PREFIX = 'Digit';

/** The lens menu (profile lenses and the user's) with filter-set chips: Alt+1 … Alt+9 switch the sets on and off. */
@Component({
  selector: 'imt-lens-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  host: { '(document:keydown)': 'onKey($event)' },
  template: `
    <div class="lens-row" role="group" aria-label="Lenses and filter sets" i18n-aria-label="@@lenses.bar">
      <label class="lens">
        <span class="faint" i18n="@@lenses.label">Lens</span>
        <select class="input" aria-label="Lens" i18n-aria-label="@@lenses.select" [value]="lenses.activeId() ?? ''" (change)="pick($event)">
          <option value="" i18n="@@lenses.none">Pick a lens…</option>
          @if (lenses.builtIn().length) {
            <optgroup label="Profile" i18n-label="@@lenses.group.profile">
              @for (l of lenses.builtIn(); track l.id) { <option [value]="l.id" [title]="l.description">{{ l.name }}</option> }
            </optgroup>
          }
          @if (lenses.saved().length) {
            <optgroup label="Saved" i18n-label="@@lenses.group.saved">
              @for (l of lenses.saved(); track l.id) { <option [value]="l.id">{{ l.name }}</option> }
            </optgroup>
          }
        </select>
      </label>
      <input class="input name" aria-label="Name" i18n-aria-label="@@lenses.name" placeholder="Name" i18n-placeholder="@@lenses.namePlaceholder" [value]="name()" (input)="name.set(val($event))" (keydown.enter)="saveLens()" />
      <button type="button" class="btn" (click)="saveLens()" title="Save the current view (query, columns, highlights) as a lens" i18n-title="@@lenses.save.title" i18n="@@lenses.save">Save lens</button>
      @if (savedActive(); as l) {
        <button type="button" class="icon-btn" [attr.aria-label]="deleteLabel(l.name)" [title]="deleteLabel(l.name)" (click)="lenses.remove(l.id)"><imt-icon name="trash" [size]="13" /></button>
      }
      <button type="button" class="btn" [disabled]="!store.query().filters.length" (click)="saveSet()" title="Move the current filters into a filter set you can switch on and off" i18n-title="@@lenses.set.title" i18n="@@lenses.set">Filters → set</button>
      @for (s of sets.sets(); track s.id; let i = $index) {
        <span class="set" [class.on]="s.enabled" data-testid="filter-set">
          <button type="button" class="toggle" [attr.aria-pressed]="s.enabled" [title]="setTitle(i)" (click)="sets.toggle(s.id)">
            <span>{{ s.name }}</span><span class="faint count">{{ s.filters.length }}</span>
          </button>
          <button type="button" class="x" [attr.aria-label]="removeSetLabel(s.name)" [attr.title]="removeSetLabel(s.name)" (click)="sets.remove(s.id)">×</button>
        </span>
      }
    </div>
  `,
  styles: `
    :host { display: block; }
    .lens-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .lens { display: flex; align-items: center; gap: 6px; }
    .name { width: 120px; }
    .set { display: inline-flex; align-items: center; height: 24px; border: 1px solid var(--border-2); border-radius: 12px; background: var(--bg-2); color: var(--faint); }
    .set.on { background: var(--bg-active); color: var(--text); border-color: var(--accent); }
    .toggle { display: inline-flex; align-items: center; gap: 6px; height: 22px; padding: 0 4px 0 10px; border: 0; background: transparent; color: inherit; cursor: pointer; font-size: var(--fs-sm); }
    .count { font-size: var(--fs-xs); }
    .x { border: 0; background: transparent; color: var(--faint); cursor: pointer; padding: 0 8px 0 2px; font-size: var(--fs-lg); line-height: 1; }
    .x:hover { color: var(--text); }
  `,
})
export class LensBar {
  protected readonly store = inject(WorkspaceStore);
  protected readonly lenses = inject(LensStore);
  protected readonly sets = inject(FilterSetStore);
  protected readonly name = signal('');
  /** The active lens when it is one the user saved (only those can be deleted). */
  protected readonly savedActive = computed(() => this.lenses.saved().find((l) => l.id === this.lenses.activeId()) ?? null);

  protected val(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }

  protected pick(e: Event): void {
    const id = (e.target as HTMLSelectElement).value;
    if (id) this.lenses.apply(id);
  }

  protected saveLens(): void {
    this.lenses.saveCurrent(this.name());
    this.name.set('');
  }

  protected saveSet(): void {
    this.lenses.saveFiltersAsSet(this.name());
    this.name.set('');
  }

  protected setTitle(index: number): string {
    return index < 9 ? $localize`:Filter set chip tooltip|@@lenses.set.toggleTitle:Switch on or off (Alt+${index + 1}:key:)` : $localize`:Filter set chip tooltip|@@lenses.set.toggleTitleNoKey:Switch on or off`;
  }
  protected deleteLabel(name: string): string {
    return $localize`:@@lenses.deleteAria:Delete lens ${name}:name:`;
  }
  protected removeSetLabel(name: string): string {
    return $localize`:@@lenses.set.removeAria:Delete filter set ${name}:name:`;
  }

  /** Alt+1 … Alt+9 toggle the nth filter set. */
  protected onKey(e: KeyboardEvent): void {
    if (!e.altKey || e.ctrlKey || e.metaKey || e.shiftKey || !e.code.startsWith(SET_KEY_PREFIX)) return;
    const n = Number(e.code.slice(SET_KEY_PREFIX.length));
    const set = n >= 1 && n <= 9 ? this.sets.sets()[n - 1] : undefined;
    if (!set) return;
    e.preventDefault();
    this.sets.toggle(set.id);
  }
}
