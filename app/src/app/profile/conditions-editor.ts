import { ChangeDetectionStrategy, Component, input, output } from '@angular/core';
import { Condition, CONDITION_OPS, ConditionOp } from '../core/profile.model';
import { Icon } from '../shared/icon';

/** "when" builder: every condition must hold (AND). */
@Component({
  selector: 'imt-conditions-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    @for (c of conditions(); track $index; let i = $index) {
      <div class="cond">
        <input class="input mono field" list="imt-profile-fields" [value]="c.field" placeholder="field" i18n-placeholder="@@profile.conditions.fieldPlaceholder" aria-label="Field" i18n-aria-label="@@profile.conditions.field" (input)="patch(i, { field: $any($event.target).value })" />
        <select class="select op" [value]="c.op" aria-label="Operator" i18n-aria-label="@@profile.conditions.operator" (change)="patch(i, { op: $any($event.target).value })">
          @for (op of ops; track op) { <option [value]="op" [selected]="op === c.op">{{ op }}</option> }
        </select>
        @if (needsValue(c.op)) {
          <input class="input mono value" [value]="c.value ?? ''" placeholder="value" i18n-placeholder="@@profile.conditions.valuePlaceholder" aria-label="Value" i18n-aria-label="@@profile.conditions.value" (input)="patch(i, { value: $any($event.target).value })" />
        } @else {
          <span class="value"></span>
        }
        <button class="icon-btn" aria-label="Remove condition" i18n-aria-label="@@profile.conditions.remove" (click)="remove(i)" title="Remove condition" i18n-title="@@profile.conditions.removeTitle"><imt-icon name="close" [size]="12" /></button>
      </div>
    } @empty {
      <div class="faint none">{{ emptyText() }}</div>
    }
    <button class="btn dashed add" (click)="add()" i18n="@@profile.conditions.add">+ Condition</button>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 4px; }
    .cond { display: grid; grid-template-columns: minmax(0, 1fr) 110px minmax(0, 1.4fr) 26px; gap: 4px; align-items: center; }
    .none { font-size: 12px; }
    .add { align-self: flex-start; }
  `,
})
export class ConditionsEditor {
  readonly conditions = input<Condition[]>([]);
  readonly emptyText = input($localize`:@@profile.conditions.everyEvent:Applies to every event.`);
  readonly changed = output<Condition[]>();
  protected readonly ops = CONDITION_OPS;

  protected needsValue(op: ConditionOp): boolean {
    return op !== 'exists' && op !== 'missing';
  }

  protected patch(i: number, p: Partial<Condition>): void {
    const next = this.conditions().map((c, j) => (j === i ? { ...c, ...p } : c));
    const target = next[i];
    if (target && p.op && !this.needsValue(p.op)) delete target.value;
    this.changed.emit(next);
  }
  protected add(): void {
    this.changed.emit([...this.conditions(), { field: '@message', op: 'contains', value: '' }]);
  }
  protected remove(i: number): void {
    this.changed.emit(this.conditions().filter((_, j) => j !== i));
  }
}
