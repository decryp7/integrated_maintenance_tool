import { ChangeDetectionStrategy, Component, computed, inject, input, output } from '@angular/core';
import { Icon } from '../shared/icon';
import { Switch } from '../shared/switch';
import { SettingsFiles } from './settings-files';
import { displayValue, editUnit, parseList, parseNumber, precheck, rangeHint, sameValue } from './settings-model';
import { SettingField, SettingValue } from './settings.types';

let nextId = 0;

/**
 * One setting, rendered from its schema: a control per kind with unit, limits, help text, a
 * "restart" hint and "Reset to default". Errors shown are the engine's (`error`), or a quick
 * range pre-check while typing.
 */
@Component({
  selector: 'imt-setting-field',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, Switch],
  host: { class: 'setting', '[class.invalid]': '!!message()', '[attr.data-key]': 'field().key' },
  template: `
    @let f = field();
    @let k = f.kind;
    <div class="head">
      @if (k.type === 'bool') {
        <imt-switch class="label" [inputId]="id" [checked]="value() === true" [disabled]="readOnly()" [describedBy]="describedBy()" [invalid]="!!message()" (checkedChange)="emit($event)">{{ f.label }}</imt-switch>
      } @else {
        <label class="label" [for]="id">{{ f.label }}</label>
      }
      @if (f.restartRequired) {
        <span class="badge" title="Takes effect after a restart">restart</span>
      }
      @if (f.advanced) {
        <span class="badge">advanced</span>
      }
      <span class="spacer"></span>
      @if (!readOnly() && !isDefault()) {
        <button class="btn ghost reset" type="button" [attr.aria-label]="'Reset ' + f.label + ' to default'" [title]="'Default: ' + defaultText()" (click)="resetToDefault.emit()">
          <imt-icon name="refresh" [size]="12" /> Reset to default
        </button>
      }
    </div>
    @if (k.type !== 'bool') {
      <div class="control">
        @switch (k.type) {
          @case ('enum') {
            <select class="select" [id]="id" [disabled]="readOnly()" [attr.aria-describedby]="describedBy()" [attr.aria-invalid]="!!message()" (change)="emit($any($event.target).value)">
              @for (o of k.options; track o.value) {
                <option [value]="o.value" [selected]="o.value === value()">{{ o.label }}</option>
              }
            </select>
          }
          @case ('text') {
            <input class="input wide" [id]="id" type="text" [value]="text()" [disabled]="readOnly()" [attr.aria-describedby]="describedBy()" [attr.aria-invalid]="!!message()" (input)="emit($any($event.target).value)" />
          }
          @case ('path') {
            <input class="input wide mono" [id]="id" type="text" [value]="text()" [disabled]="readOnly()" [placeholder]="k.dir ? 'Folder (empty = default)' : 'File (empty = default)'" [attr.aria-describedby]="describedBy()" [attr.aria-invalid]="!!message()" (input)="emit($any($event.target).value)" />
            <button class="btn" type="button" [disabled]="readOnly()" [attr.aria-label]="'Browse for ' + f.label" (click)="browse(k.dir)">Browse…</button>
          }
          @case ('stringList') {
            <textarea class="input wide" rows="3" [id]="id" [value]="listText()" [disabled]="readOnly()" [attr.aria-describedby]="describedBy()" [attr.aria-invalid]="!!message()" (input)="emit(list($any($event.target).value))"></textarea>
          }
          @default {
            <input
              class="input num"
              [id]="id"
              type="number"
              inputmode="decimal"
              [value]="numberText()"
              [attr.min]="limits().min"
              [attr.max]="limits().max"
              [attr.step]="limits().step"
              [disabled]="readOnly()"
              [attr.aria-describedby]="describedBy()"
              [attr.aria-invalid]="!!message()"
              (input)="emit(number($any($event.target).value))"
            />
            @if (unitLabel()) {
              <span class="unit">{{ unitLabel() }}</span>
            }
          }
        }
        @if (hint()) {
          <span class="hint">{{ hint() }}</span>
        }
      </div>
    }
    @if (f.description) {
      <p class="help" [id]="id + '-help'">{{ f.description }}</p>
    }
    @if (message(); as m) {
      <p class="error" [id]="id + '-error'" role="alert">{{ m }}</p>
    }
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 4px; padding: 10px 0; border-bottom: 1px solid var(--border); }
    .head { display: flex; align-items: center; gap: 8px; min-height: 26px; }
    .label { font-weight: 500; color: var(--text); min-width: 0; }
    .badge { flex: none; }
    .spacer { flex: 1; }
    .reset { height: 22px; font-size: 11px; }
    .control { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .input.num { width: 120px; }
    .input.wide, .select { min-width: 260px; max-width: 100%; }
    textarea.input.wide { width: 100%; box-sizing: border-box; font-family: var(--font-mono); }
    .unit { color: var(--text-2); }
    .hint { color: var(--muted); font-size: 12px; }
    .help { margin: 0; color: var(--muted); font-size: 12px; line-height: 1.45; }
    .error { margin: 0; color: var(--danger-text); font-size: 12px; }
    :host(.invalid) .input, :host(.invalid) .select { border-color: var(--danger-text); }
  `,
})
export class SettingFieldControl {
  private readonly files = inject(SettingsFiles);
  readonly field = input.required<SettingField>();
  readonly value = input<SettingValue | undefined>(undefined);
  /** The engine's validation message for this field. */
  readonly error = input<string | null>(null);
  readonly readOnly = input(false);
  readonly changed = output<SettingValue>();
  readonly resetToDefault = output<void>();

  protected readonly id = `imt-setting-${nextId++}`;
  protected readonly unit = computed(() => editUnit(this.field().kind));
  protected readonly unitLabel = computed(() => {
    const k = this.field().kind;
    return this.unit()?.label ?? (k.type === 'int' || k.type === 'float' ? (k.unit ?? '') : '');
  });
  protected readonly limits = computed(() => {
    const k = this.field().kind;
    const factor = this.unit()?.factor ?? 1;
    if (k.type === 'int' || k.type === 'float') return { min: k.min, max: k.max, step: k.step };
    if (k.type === 'duration') return { min: k.minMs / factor, max: k.maxMs / factor, step: 1 };
    if (k.type === 'size') return { min: k.minBytes / factor, max: k.maxBytes / factor, step: 1 };
    return { min: null, max: null, step: null };
  });
  protected readonly hint = computed(() => rangeHint(this.field().kind));
  protected readonly isDefault = computed(() => sameValue(this.value(), this.field().default));
  protected readonly defaultText = computed(() => displayValue(this.field(), this.field().default));
  /** Engine error first; otherwise the convenience pre-check. */
  protected readonly message = computed(() => this.error() ?? (this.readOnly() ? null : precheck(this.field(), this.value())));
  protected readonly describedBy = computed(() => {
    const ids = [];
    if (this.field().description) ids.push(`${this.id}-help`);
    if (this.message()) ids.push(`${this.id}-error`);
    return ids.join(' ') || null;
  });
  protected readonly text = computed(() => (typeof this.value() === 'string' ? (this.value() as string) : String(this.value() ?? '')));
  protected readonly listText = computed(() => {
    const v = this.value();
    return Array.isArray(v) ? v.join('\n') : String(v ?? '');
  });
  protected readonly numberText = computed(() => {
    const v = this.value();
    return typeof v === 'number' ? String(v / (this.unit()?.factor ?? 1)) : String(v ?? '');
  });

  protected emit(value: SettingValue): void {
    this.changed.emit(value);
  }

  protected number(text: string): SettingValue {
    return parseNumber(text, this.unit()?.factor ?? 1);
  }

  protected list(text: string): string[] {
    return parseList(text);
  }

  protected async browse(dir: boolean): Promise<void> {
    const path = await this.files.pickPath(dir, this.field().label);
    if (path) this.emit(path);
  }
}
