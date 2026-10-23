import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { formatKeybinding } from '../extensions/keybinding';
import { AutoFocus } from '../shared/auto-focus';
import { DIALOG_REF } from '../ui/dialog.service';
import { MAX_LAYOUT_NAME } from './named-layouts';
import { Perspectives } from './perspectives';

/** "Save current as perspective…": asks for a name (an existing name is replaced). */
@Component({
  selector: 'debrief-save-layout-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AutoFocus],
  template: `
    <form (submit)="$event.preventDefault(); save()">
      <label for="debrief-layout-name" i18n="@@dock.layoutDialog.name">Perspective name</label>
      <input id="debrief-layout-name" class="input" type="text" [maxLength]="max" [value]="name()" (input)="name.set($any($event.target).value)" debriefAutoFocus />
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
  private readonly perspectives = inject(Perspectives);
  protected readonly max = MAX_LAYOUT_NAME;
  protected readonly name = signal($localize`:@@dock.layoutDialog.defaultName:Perspective ${this.perspectives.list().length + 1}:n:`);
  protected readonly error = signal<string | null>(null);

  protected save(): void {
    const saved = this.perspectives.saveCurrent(this.name());
    if (saved) this.ref.close(saved);
    else this.error.set($localize`:@@dock.layoutDialog.nameRequired:Type a name for the perspective.`);
  }
}

/** "Manage perspectives…": switch, and rename, reorder (Ctrl+1…9 follow the order) and delete the saved ones. */
@Component({
  selector: 'debrief-manage-layouts-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AutoFocus],
  template: `
    <ol class="list" aria-label="Perspectives" i18n-aria-label="@@dock.layoutDialog.saved">
      @for (p of perspectives.list(); track p.id; let i = $index) {
        <li [attr.data-testid]="'perspective-row'">
          <span class="key">{{ i < 9 ? keyLabel(i + 1) : '' }}</span>
          @if (renaming() === p.name && !p.builtIn) {
            <input class="input" type="text" [attr.aria-label]="newNameLabel(p.name)" [value]="p.name" [maxLength]="60"
                   (keydown.enter)="rename(p.name, $any($event.target).value)" (keydown.escape)="$event.stopPropagation(); renaming.set(null)" debriefAutoFocus />
          } @else {
            <span class="name">{{ p.name }}@if (p.id === perspectives.active().id) { <span class="current" i18n="@@dock.layoutDialog.current">(current)</span> }</span>
            <span class="when">{{ p.builtIn ? p.description : '' }}</span>
          }
          <button type="button" class="btn" [disabled]="p.id === perspectives.active().id" (click)="apply(p.id)" [attr.aria-label]="applyLabel(p.name)" i18n="@@dock.layoutDialog.apply">Switch</button>
          @if (!p.builtIn) {
            <button type="button" class="btn ghost" (click)="renaming.set(p.name)" [attr.aria-label]="renameLabel(p.name)" i18n="@@dock.layoutDialog.rename">Rename</button>
            <button type="button" class="btn ghost" [disabled]="!canMove(p.name, -1)" (click)="perspectives.move(p.name, -1)" [attr.aria-label]="moveUpLabel(p.name)">↑</button>
            <button type="button" class="btn ghost" [disabled]="!canMove(p.name, 1)" (click)="perspectives.move(p.name, 1)" [attr.aria-label]="moveDownLabel(p.name)">↓</button>
            <button type="button" class="btn ghost" (click)="perspectives.remove(p.name)" [attr.aria-label]="deleteLabel(p.name)" i18n="@@dock.layoutDialog.delete">Delete</button>
          }
        </li>
      }
    </ol>
    <div class="foot">
      <button type="button" class="btn ghost" (click)="reset()" i18n="@@dock.layoutDialog.reset">Reset perspective</button>
      <button type="button" class="btn primary" (click)="ref.close()" i18n="@@dock.layoutDialog.close">Close</button>
    </div>
  `,
  styles: `
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
    li { display: flex; align-items: center; gap: 6px; }
    .key { width: 64px; font-size: var(--fs-xs); color: var(--muted); }
    .name { min-width: 120px; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .current { color: var(--muted); font-size: var(--fs-xs); }
    li .input { flex: 1; }
    .when { flex: 1; font-size: var(--fs-xs); color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .foot { display: flex; justify-content: space-between; gap: 8px; margin-top: 12px; }
  `,
})
export class ManageLayoutsDialog {
  protected readonly ref = inject(DIALOG_REF);
  protected readonly perspectives = inject(Perspectives);
  protected readonly renaming = signal<string | null>(null);

  protected keyLabel(n: number): string {
    return formatKeybinding(`Mod+${n}`);
  }

  /** Whether a saved perspective can move up (-1) or down (+1) among the saved ones. */
  protected canMove(name: string, delta: -1 | 1): boolean {
    const saved = this.perspectives.list().filter((p) => !p.builtIn);
    const i = saved.findIndex((p) => p.name === name);
    return i + delta >= 0 && i + delta < saved.length;
  }

  protected newNameLabel(name: string): string {
    return $localize`:@@dock.layoutDialog.newNameLabel:New name for ${name}:name:`;
  }

  protected applyLabel(name: string): string {
    return $localize`:@@dock.layoutDialog.applyLabel:Switch to ${name}:name:`;
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

  protected apply(id: string): void {
    if (this.perspectives.switchTo(id)) this.ref.close();
  }

  protected rename(from: string, to: string): void {
    this.perspectives.rename(from, to);
    this.renaming.set(null);
  }

  protected reset(): void {
    this.perspectives.reset();
    this.ref.close();
  }
}
