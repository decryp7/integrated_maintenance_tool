import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { HighlightOp } from '../../core/tools.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { Icon } from '../../shared/icon';
import { HIGHLIGHT_COLORS, HIGHLIGHT_OPS, HighlightRules } from './highlight';

const PSEUDO_FIELDS = ['@level', '@message', '@exception', '@source'];

/** Highlight rules: "when this field matches, colour the row". The grid reads `HighlightRules.colorFor`. */
@Component({
  selector: 'imt-highlight-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <div class="add">
      <input class="input" list="imt-hl-fields" aria-label="Highlight field" placeholder="Field, e.g. @level" [value]="field()" (input)="field.set(val($event))" />
      <datalist id="imt-hl-fields">@for (f of fields(); track f) { <option [value]="f"></option> }</datalist>
      <select class="select" aria-label="Highlight condition" [value]="op()" (change)="op.set(opOf($event))">
        @for (o of ops; track o.id) { <option [value]="o.id">{{ o.label }}</option> }
      </select>
      @if (needsValue()) {
        <input class="input" aria-label="Highlight value" placeholder="Value" [value]="value()" (input)="value.set(val($event))" />
      }
      <span class="swatches" role="radiogroup" aria-label="Row colour">
        @for (c of colors; track c) {
          <button type="button" class="sw" role="radio" [attr.aria-checked]="color() === c" [attr.aria-label]="'Colour ' + c" [style.background]="c" [class.on]="color() === c" (click)="color.set(c)"></button>
        }
      </span>
      <button type="button" class="btn" [disabled]="!field().trim()" (click)="add()">Add rule</button>
    </div>
    @if (rules.items().length) {
      <ul class="list" aria-label="Highlight rules">
        @for (r of rules.items(); track r.id) {
          <li class="item">
            <input type="checkbox" [checked]="r.enabled" [attr.aria-label]="'Enable rule ' + r.field" (change)="rules.update(r.id, { enabled: checked($event) })" />
            <span class="sw static" [style.background]="r.color" aria-hidden="true"></span>
            <span class="grow ellipsis"><span class="mono">{{ r.field }}</span> {{ opLabel(r.op) }} <span class="mono">{{ r.value }}</span></span>
            <button type="button" class="icon-btn" [attr.aria-label]="'Delete rule ' + r.field" title="Delete" (click)="rules.remove(r.id)"><imt-icon name="trash" [size]="13" /></button>
          </li>
        }
      </ul>
    } @else {
      <p class="faint">No highlight rules.</p>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; }
    .add { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .swatches { display: inline-flex; gap: 4px; }
    .sw { width: 18px; height: 18px; border-radius: 50%; border: 2px solid transparent; padding: 0; cursor: pointer; }
    .sw.on { border-color: var(--text); }
    .sw.static { width: 12px; height: 12px; flex: none; cursor: default; }
    .grow { flex: 1; min-width: 0; }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
    .item { display: flex; align-items: center; gap: 8px; padding: 4px 8px; border: 1px solid var(--border-2); border-radius: 6px; background: var(--bg-2); }
    p { margin: 0; }
  `,
})
export class HighlightEditor {
  protected readonly rules = inject(HighlightRules);
  private readonly workspace = inject(WorkspaceStore);
  protected readonly ops = HIGHLIGHT_OPS;
  protected readonly colors = HIGHLIGHT_COLORS;

  protected readonly field = signal('');
  protected readonly op = signal<HighlightOp>('contains');
  protected readonly value = signal('');
  protected readonly color = signal(HIGHLIGHT_COLORS[0]);
  protected readonly fields = computed(() => [...PSEUDO_FIELDS, ...this.workspace.fieldNames()]);
  protected readonly needsValue = computed(() => HIGHLIGHT_OPS.find((o) => o.id === this.op())?.needsValue ?? true);

  protected add(): void {
    this.rules.add({ field: this.field().trim(), op: this.op(), value: this.needsValue() ? this.value() : '', color: this.color() });
    this.field.set('');
    this.value.set('');
  }
  protected opLabel = (op: HighlightOp) => HIGHLIGHT_OPS.find((o) => o.id === op)?.label ?? op;
  protected val = (e: Event) => (e.target as HTMLInputElement).value;
  protected opOf = (e: Event) => this.val(e) as HighlightOp;
  protected checked = (e: Event) => (e.target as HTMLInputElement).checked;
}
