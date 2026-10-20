import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { DatePipe } from '@angular/common';
import { AutoFocus } from '../shared/auto-focus';
import { DIALOG_REF } from '../ui/dialog.service';
import { DockService } from './dock.service';
import { MAX_LAYOUT_NAME } from './named-layouts';
import { WindowLayouts } from './window-layouts';

/** "Save window layout…": asks for a name (an existing name is replaced). */
@Component({
  selector: 'imt-save-layout-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AutoFocus],
  template: `
    <form (submit)="$event.preventDefault(); save()">
      <label for="imt-layout-name" i18n="@@dock.layoutDialog.name">Layout name</label>
      <input id="imt-layout-name" class="input" type="text" [maxLength]="max" [value]="name()" (input)="name.set($any($event.target).value)" imtAutoFocus />
      @if (error()) {
        <p class="error" role="alert">{{ error() }}</p>
      }
      <div class="foot">
        <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@dock.layoutDialog.cancel">Cancel</button>
        <button type="submit" class="btn primary" i18n="@@dock.layoutDialog.save">Save</button>
      </div>
    </form>
  `,
  styles: `
    form { display: flex; flex-direction: column; gap: 8px; }
    label { font-size: var(--fs-sm); color: var(--muted); }
    .error { margin: 0; color: var(--danger-text); font-size: var(--fs-sm); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  `,
})
export class SaveLayoutDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly layouts = inject(WindowLayouts);
  protected readonly max = MAX_LAYOUT_NAME;
  protected readonly name = signal($localize`:@@dock.layoutDialog.defaultName:Layout ${this.layouts.list().length + 1}:n:`);
  protected readonly error = signal<string | null>(null);

  protected save(): void {
    const saved = this.layouts.saveCurrent(this.name());
    if (saved) this.ref.close(saved);
    else this.error.set($localize`:@@dock.layoutDialog.nameRequired:Type a name for the layout.`);
  }
}

/** "Manage window layouts…": apply, rename, reorder (the first four have Ctrl+Alt+1..4) and delete. */
@Component({
  selector: 'imt-manage-layouts-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [DatePipe, AutoFocus],
  template: `
    @if (layouts.list().length) {
      <ol class="list" aria-label="Saved window layouts" i18n-aria-label="@@dock.layoutDialog.saved">
        @for (l of layouts.list(); track l.name; let i = $index) {
          <li>
            <span class="key">{{ i < 4 ? 'Ctrl+Alt+' + (i + 1) : '' }}</span>
            @if (renaming() === l.name) {
              <input class="input" type="text" [attr.aria-label]="newNameLabel(l.name)" [value]="l.name" [maxLength]="60"
                     (keydown.enter)="rename(l.name, $any($event.target).value)" (keydown.escape)="$event.stopPropagation(); renaming.set(null)" imtAutoFocus />
            } @else {
              <span class="name">{{ l.name }}</span>
              <span class="when">{{ l.savedAt | date: 'short' }}</span>
            }
            <button type="button" class="btn" (click)="apply(l.name)" [attr.aria-label]="applyLabel(l.name)" i18n="@@dock.layoutDialog.apply">Apply</button>
            <button type="button" class="btn ghost" (click)="renaming.set(l.name)" [attr.aria-label]="renameLabel(l.name)" i18n="@@dock.layoutDialog.rename">Rename</button>
            <button type="button" class="btn ghost" [disabled]="i === 0" (click)="layouts.move(l.name, -1)" [attr.aria-label]="moveUpLabel(l.name)">↑</button>
            <button type="button" class="btn ghost" [disabled]="i === layouts.list().length - 1" (click)="layouts.move(l.name, 1)" [attr.aria-label]="moveDownLabel(l.name)">↓</button>
            <button type="button" class="btn ghost" (click)="layouts.remove(l.name)" [attr.aria-label]="deleteLabel(l.name)" i18n="@@dock.layoutDialog.delete">Delete</button>
          </li>
        }
      </ol>
    } @else {
      <p class="empty" i18n="@@dock.layoutDialog.empty">No saved window layouts yet. Use Window › Save window layout… to save the current arrangement.</p>
    }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="reset()" i18n="@@dock.layoutDialog.reset">Reset window layout</button>
      <button type="button" class="btn primary" (click)="ref.close()" i18n="@@dock.layoutDialog.close">Close</button>
    </div>
  `,
  styles: `
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
    li { display: flex; align-items: center; gap: 6px; }
    .key { width: 72px; font-size: var(--fs-xs); color: var(--muted); }
    .name { flex: 1; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    li .input { flex: 1; }
    .when { font-size: var(--fs-xs); color: var(--muted); }
    .empty { margin: 0; color: var(--muted); }
    .foot { display: flex; justify-content: space-between; gap: 8px; margin-top: 12px; }
  `,
})
export class ManageLayoutsDialog {
  protected readonly ref = inject(DIALOG_REF);
  protected readonly layouts = inject(WindowLayouts);
  private readonly dock = inject(DockService);
  protected readonly renaming = signal<string | null>(null);

  protected newNameLabel(name: string): string {
    return $localize`:@@dock.layoutDialog.newNameLabel:New name for ${name}:name:`;
  }

  protected applyLabel(name: string): string {
    return $localize`:@@dock.layoutDialog.applyLabel:Apply ${name}:name:`;
  }

  protected renameLabel(name: string): string {
    return $localize`:@@dock.layoutDialog.renameLabel:Rename ${name}:name:`;
  }

  protected moveUpLabel(name: string): string {
    return $localize`:@@dock.layoutDialog.moveUpLabel:Move ${name}:name: up`;
  }

  protected moveDownLabel(name: string): string {
    return $localize`:@@dock.layoutDialog.moveDownLabel:Move ${name}:name: down`;
  }

  protected deleteLabel(name: string): string {
    return $localize`:@@dock.layoutDialog.deleteLabel:Delete ${name}:name:`;
  }

  protected apply(name: string): void {
    if (this.layouts.apply(name)) this.ref.close();
  }

  protected rename(from: string, to: string): void {
    this.layouts.rename(from, to);
    this.renaming.set(null);
  }

  protected reset(): void {
    this.dock.reset();
    this.ref.close();
  }
}
