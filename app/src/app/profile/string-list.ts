import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';
import { Icon } from '../shared/icon';

/** Editable list of strings (globs, templates, regexes, field names). */
@Component({
  selector: 'debrief-string-list',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    @for (v of values(); track $index; let i = $index) {
      <div class="item">
        <input
          class="input"
          [class.mono]="mono()"
          [value]="v"
          [attr.aria-label]="label() + ' ' + (i + 1)"
          [attr.list]="datalist() || null"
          (input)="set(i, $any($event.target).value)"
        />
        <button class="icon-btn" [attr.aria-label]="moveUpLabel(i)" [disabled]="i === 0" (click)="move(i, -1)" [attr.title]="moveUpLabel(i)">↑</button>
        <button class="icon-btn" [attr.aria-label]="removeLabel(i)" (click)="remove(i)" [attr.title]="removeLabel(i)"><debrief-icon name="close" [size]="12" /></button>
      </div>
    }
    <button class="btn dashed add" (click)="add()" i18n="@@profile.stringList.add">+ Add {{ label() }}</button>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 4px; }
    .item { display: flex; gap: 4px; align-items: center; }
    .item .input { flex: 1; }
    .icon-btn:disabled { opacity: 0.3; cursor: default; }
    .add { align-self: flex-start; }
  `,
})
export class StringList {
  readonly values = input<string[]>([]);
  readonly label = input($localize`:@@profile.stringList.item:item`);
  readonly mono = input(true);
  readonly datalist = input('');
  readonly changed = output<string[]>();

  protected moveUpLabel(i: number): string {
    const [label, n] = [this.label(), i + 1];
    return $localize`:@@profile.stringList.moveUp:Move ${label}:label: ${n}:number: up`;
  }
  protected removeLabel(i: number): string {
    const [label, n] = [this.label(), i + 1];
    return $localize`:@@profile.stringList.remove:Remove ${label}:label: ${n}:number:`;
  }
  protected set(i: number, v: string): void {
    const next = [...this.values()];
    next[i] = v;
    this.changed.emit(next);
  }
  protected add(): void {
    this.changed.emit([...this.values(), '']);
  }
  protected remove(i: number): void {
    this.changed.emit(this.values().filter((_, j) => j !== i));
  }
  protected move(i: number, d: number): void {
    const next = [...this.values()];
    const moved = next.splice(i, 1);
    next.splice(i + d, 0, ...moved);
    this.changed.emit(next);
  }
}
