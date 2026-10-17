import { ChangeDetectionStrategy, Component, inject, signal } from '@angular/core';
import { DatePipe } from '@angular/common';
import { DIALOG_REF } from '../ui/dialog.service';
import { DockService } from './dock.service';
import { MAX_LAYOUT_NAME } from './named-layouts';
import { WindowLayouts } from './window-layouts';

/** "Save window layout…": asks for a name (an existing name is replaced). */
@Component({
  selector: 'imt-save-layout-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <form (submit)="$event.preventDefault(); save()">
      <label for="imt-layout-name">Layout name</label>
      <input id="imt-layout-name" class="input" type="text" [maxLength]="max" [value]="name()" (input)="name.set($any($event.target).value)" autofocus />
      @if (error()) {
        <p class="error" role="alert">{{ error() }}</p>
      }
      <div class="foot">
        <button type="button" class="btn ghost" (click)="ref.close()">Cancel</button>
        <button type="submit" class="btn primary">Save</button>
      </div>
    </form>
  `,
  styles: `
    form { display: flex; flex-direction: column; gap: 8px; }
    label { font-size: 12px; color: var(--muted); }
    .error { margin: 0; color: var(--danger-text); font-size: 12px; }
    .foot { display: flex; justify-content: flex-end; gap: 8px; margin-top: 4px; }
  `,
})
export class SaveLayoutDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly layouts = inject(WindowLayouts);
  protected readonly max = MAX_LAYOUT_NAME;
  protected readonly name = signal(`Layout ${this.layouts.list().length + 1}`);
  protected readonly error = signal<string | null>(null);

  protected save(): void {
    const saved = this.layouts.saveCurrent(this.name());
    if (saved) this.ref.close(saved);
    else this.error.set('Type a name for the layout.');
  }
}

/** "Manage window layouts…": apply, rename, reorder (the first four have Ctrl+Alt+1..4) and delete. */
@Component({
  selector: 'imt-manage-layouts-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [DatePipe],
  template: `
    @if (layouts.list().length) {
      <ol class="list" aria-label="Saved window layouts">
        @for (l of layouts.list(); track l.name; let i = $index) {
          <li>
            <span class="key">{{ i < 4 ? 'Ctrl+Alt+' + (i + 1) : '' }}</span>
            @if (renaming() === l.name) {
              <input class="input" type="text" [attr.aria-label]="'New name for ' + l.name" [value]="l.name" [maxLength]="60"
                     (keydown.enter)="rename(l.name, $any($event.target).value)" (keydown.escape)="$event.stopPropagation(); renaming.set(null)" autofocus />
            } @else {
              <span class="name">{{ l.name }}</span>
              <span class="when">{{ l.savedAt | date: 'short' }}</span>
            }
            <button type="button" class="btn" (click)="apply(l.name)" [attr.aria-label]="'Apply ' + l.name">Apply</button>
            <button type="button" class="btn ghost" (click)="renaming.set(l.name)" [attr.aria-label]="'Rename ' + l.name">Rename</button>
            <button type="button" class="btn ghost" [disabled]="i === 0" (click)="layouts.move(l.name, -1)" [attr.aria-label]="'Move ' + l.name + ' up'">↑</button>
            <button type="button" class="btn ghost" [disabled]="i === layouts.list().length - 1" (click)="layouts.move(l.name, 1)" [attr.aria-label]="'Move ' + l.name + ' down'">↓</button>
            <button type="button" class="btn ghost" (click)="layouts.remove(l.name)" [attr.aria-label]="'Delete ' + l.name">Delete</button>
          </li>
        }
      </ol>
    } @else {
      <p class="empty">No saved window layouts yet. Use Window › Save window layout… to save the current arrangement.</p>
    }
    <div class="foot">
      <button type="button" class="btn ghost" (click)="reset()">Reset window layout</button>
      <button type="button" class="btn primary" (click)="ref.close()">Close</button>
    </div>
  `,
  styles: `
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
    li { display: flex; align-items: center; gap: 6px; }
    .key { width: 72px; font-size: 11px; color: var(--muted); }
    .name { flex: 1; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    li .input { flex: 1; }
    .when { font-size: 11px; color: var(--muted); }
    .empty { margin: 0; color: var(--muted); }
    .foot { display: flex; justify-content: space-between; gap: 8px; margin-top: 12px; }
  `,
})
export class ManageLayoutsDialog {
  protected readonly ref = inject(DIALOG_REF);
  protected readonly layouts = inject(WindowLayouts);
  private readonly dock = inject(DockService);
  protected readonly renaming = signal<string | null>(null);

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
