import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';
import { Icon } from '../shared/icon';

/** Editable list of strings (globs, templates, regexes, field names). */
@Component({
  selector: 'imt-string-list',
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
        <button class="icon-btn" [attr.aria-label]="'Move ' + label() + ' ' + (i + 1) + ' up'" [disabled]="i === 0" (click)="move(i, -1)">↑</button>
        <button class="icon-btn" [attr.aria-label]="'Remove ' + label() + ' ' + (i + 1)" (click)="remove(i)"><imt-icon name="close" [size]="12" /></button>
      </div>
    }
    <button class="btn dashed add" (click)="add()">+ Add {{ label() }}</button>
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
  readonly label = input('item');
  readonly mono = input(true);
  readonly datalist = input('');
  readonly changed = output<string[]>();

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
    const [x] = next.splice(i, 1);
    next.splice(i + d, 0, x);
    this.changed.emit(next);
  }
}
