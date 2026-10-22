import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { HighlightOp } from '../../core/tools.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { Icon } from '../../shared/icon';
import { HIGHLIGHT_COLORS, HIGHLIGHT_OPS, HighlightRules } from './highlight';

const PSEUDO_FIELDS = ['@level', '@message', '@exception', '@source'];

/** Highlight rules: "when this field matches, colour the row". The grid reads `HighlightRules.colorFor`. */
@Component({
  selector: 'debrief-highlight-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon],
  template: `
    <div class="add">
      <input class="input" list="debrief-hl-fields" aria-label="Highlight field" i18n-aria-label="@@projects.highlightEditor.field" placeholder="Field, e.g. @level" i18n-placeholder="@@projects.highlightEditor.fieldPlaceholder" [value]="field()" (input)="field.set(val($event))" />
      <datalist id="debrief-hl-fields">@for (f of fields(); track f) { <option [value]="f"></option> }</datalist>
      <select class="select" aria-label="Highlight condition" i18n-aria-label="@@projects.highlightEditor.condition" [value]="op()" (change)="op.set(opOf($event))">
        @for (o of ops; track o.id) { <option [value]="o.id">{{ o.label }}</option> }
      </select>
      @if (needsValue()) {
        <input class="input" aria-label="Highlight value" i18n-aria-label="@@projects.highlightEditor.value" placeholder="Value" i18n-placeholder="@@projects.highlightEditor.valuePlaceholder" [value]="value()" (input)="value.set(val($event))" />
      }
      <span class="swatches" role="radiogroup" aria-label="Row colour" i18n-aria-label="@@projects.highlightEditor.rowColour">
        @for (c of colors; track c) {
          <button type="button" class="sw" role="radio" [attr.aria-checked]="color() === c" [attr.aria-label]="colourLabel(c)" [class.on]="color() === c" (click)="color.set(c)"><span class="dot" [style.background]="c"></span></button>
        }
      </span>
      <button type="button" class="btn" [disabled]="!field().trim()" (click)="add()" i18n="@@projects.highlightEditor.add">Add rule</button>
    </div>
    @if (rules.items().length) {
      <ul class="list" aria-label="Highlight rules" i18n-aria-label="@@projects.highlightEditor.list">
        @for (r of rules.items(); track r.id) {
          <li class="item">
            <input type="checkbox" [checked]="r.enabled" [attr.aria-label]="enableLabel(r.field)" (change)="rules.update(r.id, { enabled: checked($event) })" />
            <span class="sw static" [style.background]="r.color" aria-hidden="true"></span>
            <span class="grow ellipsis"><span class="mono">{{ r.field }}</span> {{ opLabel(r.op) }} <span class="mono">{{ r.value }}</span></span>
            <button type="button" class="icon-btn" [attr.aria-label]="deleteLabel(r.field)" title="Delete" i18n-title="@@projects.highlightEditor.delete" (click)="rules.remove(r.id)"><debrief-icon name="trash" [size]="13" /></button>
          </li>
        }
      </ul>
    } @else {
      <p class="faint" i18n="@@projects.highlightEditor.empty">No highlight rules.</p>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; }
    .add { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .swatches { display: inline-flex; gap: 0; }
    /* Each swatch is a transparent 24px target (WCAG 2.5.8) around the visible 18px dot, which carries the focus ring. */
    .sw { display: inline-flex; align-items: center; justify-content: center; width: 24px; height: 24px; border: 0; padding: 0; background: transparent; cursor: pointer; }
    .sw:focus-visible { outline: none; }
    .sw:focus-visible .dot { outline: 2px solid var(--focus-ring); outline-offset: 1px; }
    .dot { display: block; width: 18px; height: 18px; border-radius: 50%; border: 2px solid transparent; }
    .sw.on .dot { border-color: var(--text); }
    .sw.static { display: inline-block; width: 12px; height: 12px; border-radius: 50%; flex: none; cursor: default; }
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
  protected readonly color = signal(HIGHLIGHT_COLORS[0] ?? '');
  protected readonly fields = computed(() => [...PSEUDO_FIELDS, ...this.workspace.fieldNames()]);
  protected readonly needsValue = computed(() => HIGHLIGHT_OPS.find((o) => o.id === this.op())?.needsValue ?? true);

  protected add(): void {
    this.rules.add({ field: this.field().trim(), op: this.op(), value: this.needsValue() ? this.value() : '', color: this.color() });
    this.field.set('');
    this.value.set('');
  }
  protected colourLabel(colour: string): string {
    return $localize`:@@projects.highlightEditor.colourAria:Colour ${colour}:colour:`;
  }
  protected enableLabel(field: string): string {
    return $localize`:@@projects.highlightEditor.enableAria:Enable rule ${field}:field:`;
  }
  protected deleteLabel(field: string): string {
    return $localize`:@@projects.highlightEditor.deleteAria:Delete rule ${field}:field:`;
  }
  protected opLabel = (op: HighlightOp) => HIGHLIGHT_OPS.find((o) => o.id === op)?.label ?? op;
  protected val = (e: Event) => (e.target as HTMLInputElement).value;
  protected opOf = (e: Event) => this.val(e) as HighlightOp;
  protected checked = (e: Event) => (e.target as HTMLInputElement).checked;
}
