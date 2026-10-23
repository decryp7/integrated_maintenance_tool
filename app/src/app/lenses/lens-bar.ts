import { CdkMenu, CdkMenuItem, CdkMenuTrigger } from '@angular/cdk/menu';
import { ChangeDetectionStrategy, Component, computed, ElementRef, inject, signal, viewChild } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { ClickOutside } from '../shared/click-outside';
import { Icon } from '../shared/icon';
import { FilterSetStore } from './filter-set.store';
import { LensStore } from './lens.store';

/** Number keys that toggle the first nine filter sets together with Alt (`event.code`, so the keyboard layout does not matter). */
export const SET_KEY_PREFIX = 'Digit';

/** One ghost "Lens" button opening a menu (profile and saved lenses, save, filters → set) with filter-set chips: Alt+1 … Alt+9 switch the sets on and off. */
@Component({
  selector: 'debrief-lens-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, ClickOutside, CdkMenu, CdkMenuItem, CdkMenuTrigger],
  host: { '(document:keydown)': 'onKey($event)' },
  template: `
    <div class="anchor" (debriefClickOutside)="saving.set(false)" (keydown.escape)="saving.set(false)">
      <button type="button" class="btn ghost lens-btn" data-testid="lens-button" [cdkMenuTriggerFor]="menu" [title]="buttonTitle()">
        <debrief-icon name="search" [size]="14" />
        <span class="lens-name" data-testid="lens-name">{{ buttonLabel() }}</span>
        <debrief-icon name="chevronDown" [size]="12" />
      </button>
      @if (saving()) {
        <form class="popover save-pop" data-testid="lens-save-form" (submit)="$event.preventDefault(); saveLens()">
          <label class="lbl"><ng-container i18n="@@lenses.name">Name</ng-container>
            <input class="input" #nameInput [value]="name()" (input)="name.set(val($event))" placeholder="Lens name" i18n-placeholder="@@lenses.namePlaceholder" />
          </label>
          <div class="pop-actions">
            <button type="button" class="btn ghost" (click)="saving.set(false)" i18n="@@lenses.cancel">Cancel</button>
            <button type="submit" class="btn primary" i18n="@@lenses.save">Save lens</button>
          </div>
        </form>
      }
    </div>
    @for (s of sets.sets(); track s.id; let i = $index) {
      <span class="set" [class.on]="s.enabled" data-testid="filter-set">
        <button type="button" class="toggle" [attr.aria-pressed]="s.enabled" [title]="setTitle(i)" (click)="sets.toggle(s.id)">
          <span>{{ s.name }}</span><span class="faint count">{{ s.filters.length }}</span>
        </button>
        <button type="button" class="x" [attr.aria-label]="removeSetLabel(s.name)" [attr.title]="removeSetLabel(s.name)" (click)="sets.remove(s.id)">×</button>
      </span>
    }

    <ng-template #menu>
      <div cdkMenu class="menu" aria-label="Lenses" i18n-aria-label="@@lenses.menu" data-testid="lens-menu">
        @if (lenses.builtIn().length) {
          <span class="group" i18n="@@lenses.group.profile">Profile</span>
          @for (l of lenses.builtIn(); track l.id) {
            <button type="button" cdkMenuItem class="mi" [title]="l.description" [attr.data-testid]="'lens-item'" (cdkMenuItemTriggered)="lenses.apply(l.id)">
              <span class="check" aria-hidden="true">{{ lenses.activeId() === l.id ? '●' : '' }}</span>{{ l.name }}
            </button>
          }
        }
        @if (lenses.saved().length) {
          <span class="group" i18n="@@lenses.group.saved">Saved</span>
          @for (l of lenses.saved(); track l.id) {
            <button type="button" cdkMenuItem class="mi" [attr.data-testid]="'lens-item'" (cdkMenuItemTriggered)="lenses.apply(l.id)">
              <span class="check" aria-hidden="true">{{ lenses.activeId() === l.id ? '●' : '' }}</span>{{ l.name }}
            </button>
          }
        }
        <hr class="sep" />
        <button type="button" cdkMenuItem class="mi" data-testid="lens-save-item" title="Save the current view (query, columns, highlights) as a lens" i18n-title="@@lenses.save.title" (cdkMenuItemTriggered)="startSave()">
          <span class="check"></span><ng-container i18n="@@lenses.saveAs">Save current as lens…</ng-container>
        </button>
        <button type="button" cdkMenuItem class="mi" data-testid="lens-set-item" [cdkMenuItemDisabled]="!store.query().filters.length" title="Move the current filters into a filter set you can switch on and off" i18n-title="@@lenses.set.title" (cdkMenuItemTriggered)="saveSet()">
          <span class="check"></span><ng-container i18n="@@lenses.set">Filters → set</ng-container>
        </button>
        @if (savedActive(); as l) {
          <button type="button" cdkMenuItem class="mi danger" data-testid="lens-delete-item" (cdkMenuItemTriggered)="lenses.remove(l.id)">
            <span class="check"></span>{{ deleteLabel(l.name) }}
          </button>
        }
      </div>
    </ng-template>
  `,
  styles: `
    /* The host adds no box of its own: the button and the filter-set chips sit in the filter row. */
    :host { display: contents; }
    .anchor { position: relative; flex: none; }
    .lens-btn { height: 30px; gap: 6px; max-width: 200px; }
    .lens-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .save-pop { top: 34px; right: 0; width: 240px; display: flex; flex-direction: column; gap: 8px; }
    .lbl { display: flex; flex-direction: column; gap: 4px; color: var(--muted); font-size: var(--fs-sm); }
    .pop-actions { display: flex; justify-content: flex-end; gap: 6px; }
    .menu { min-width: 220px; max-width: min(360px, calc(100vw - 16px)); display: flex; flex-direction: column; padding: 4px; border-radius: 8px; background: var(--bg-2); border: 1px solid var(--border-3); box-shadow: var(--shadow); }
    .menu:focus { outline: none; }
    .group { padding: 6px 10px 2px 22px; color: var(--faint); font-size: var(--fs-xs); font-weight: 500; }
    .mi { display: flex; align-items: center; gap: 6px; padding: 5px 10px 5px 6px; border: 0; border-radius: 5px; background: transparent; color: var(--text); cursor: pointer; text-align: start; font: inherit; }
    .mi:hover:not([aria-disabled='true']), .mi:focus { background: var(--bg-active); outline: none; }
    .mi:focus-visible { box-shadow: inset 0 0 0 1px var(--accent); }
    .mi[aria-disabled='true'] { color: var(--faint); cursor: default; }
    .mi.danger { color: var(--danger-text, var(--lvl-error)); }
    .check { width: 14px; flex: none; text-align: center; color: var(--accent); font-size: var(--fs-xs); }
    .sep { border: 0; border-top: 1px solid var(--border-2); margin: 4px 0; width: 100%; }
    .set { display: inline-flex; align-items: center; height: 24px; border: 1px solid var(--border-2); border-radius: 12px; background: var(--bg-2); color: var(--faint); flex: none; }
    .set.on { background: var(--bg-active); color: var(--text); border-color: var(--accent); }
    .toggle { display: inline-flex; align-items: center; gap: 6px; height: 22px; padding: 0 4px 0 10px; border: 0; background: transparent; color: inherit; cursor: pointer; font-size: var(--fs-sm); }
    .count { font-size: var(--fs-xs); font-variant-numeric: tabular-nums; }
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

  protected readonly saving = signal(false);
  private readonly nameInput = viewChild<ElementRef<HTMLInputElement>>('nameInput');
  /** The button's text: the active lens, or "No lens". */
  protected readonly buttonLabel = computed(() => this.lenses.active()?.name ?? $localize`:@@lenses.noLens:No lens`);
  protected readonly buttonTitle = computed(() => $localize`:@@lenses.button.title:Lens: ${this.buttonLabel()}:name:`);

  /** Opens the small name popover (the menu has closed by now). */
  protected startSave(): void {
    this.saving.set(true);
    setTimeout(() => this.nameInput()?.nativeElement.focus());
  }

  protected saveLens(): void {
    this.lenses.saveCurrent(this.name());
    this.name.set('');
    this.saving.set(false);
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
