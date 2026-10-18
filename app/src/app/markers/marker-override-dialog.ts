import { ChangeDetectionStrategy, Component, computed, inject, input, linkedSignal } from '@angular/core';
import { MARKER_SHAPES } from '../core/api.types';
import { DIALOG_FORM_STYLES } from '../context/dialog-styles';
import { DIALOG_REF, DialogRef } from '../ui/dialog.service';
import { COLOR_PRESETS } from './marker-editor';
import { MarkerGlyph } from './marker-glyph';
import { OverrideStyle, PluginKindRow, styleProblems } from './marker-overrides';

/**
 * Restyles a plugin marker kind: label, description, colour and badge are written as a `[[marker_override]]`
 * (the plugin's rule itself stays read-only). Closes with the style, or undefined when cancelled.
 */
@Component({
  selector: 'imt-marker-override-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkerGlyph],
  template: `
    <p class="faint" i18n="@@markers.override.intro">This marker belongs to the plugin <span class="mono">{{ row().owner }}</span>. Its rule cannot be edited; you can change how it looks. Empty fields keep the plugin's style.</p>
    <div class="top">
      <imt-marker-glyph [glyph]="preview()" [size]="32" [label]="previewLabel" />
      <div class="grow">
        <label class="lbl"><ng-container i18n="@@markers.override.label">Label</ng-container><input class="input" name="label" [value]="label()" [placeholder]="row().info?.label ?? ''" (input)="label.set($any($event.target).value)" /></label>
        <label class="lbl"><ng-container i18n="@@markers.override.description">Description</ng-container><input class="input" name="description" [value]="description()" (input)="description.set($any($event.target).value)" /></label>
      </div>
    </div>
    <div class="row">
      <label class="lbl"><ng-container i18n="@@markers.override.colour">Colour</ng-container><input class="input mono" name="color" [value]="color()" placeholder="#RRGGBB" (input)="color.set($any($event.target).value)" /></label>
      <div class="presets" role="group" aria-label="Colour presets" i18n-aria-label="@@markers.override.colourPresets">
        @for (p of presets; track p) {
          <button type="button" class="swatch" [style.background]="p" [attr.aria-label]="colourLabel(p)" [attr.aria-pressed]="color() === p" (click)="color.set(p)"></button>
        }
      </div>
    </div>
    <div class="row">
      <label class="lbl"><ng-container i18n="@@markers.override.badge">Badge (1–3 characters)</ng-container><input class="input mono" name="badge" [value]="badge()" (input)="badge.set($any($event.target).value)" /></label>
      <label class="lbl"><ng-container i18n="@@markers.override.shape">Shape</ng-container>
        <select class="input" name="shape" [value]="shape()" (change)="shape.set($any($event.target).value)">
          <option value="" i18n="@@markers.override.pluginDefault">Plugin default</option>
          @for (s of shapes; track s) { <option [value]="s">{{ s }}</option> }
        </select>
      </label>
    </div>
    @for (p of problems(); track p) { <div class="err" role="alert">{{ p }}</div> }
    <div class="actions">
      <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@markers.override.cancel">Cancel</button>
      <button type="button" class="btn primary" [disabled]="problems().length > 0" (click)="save()" i18n="@@markers.override.save">Save style</button>
    </div>
  `,
  styles: [
    DIALOG_FORM_STYLES,
    `
      .top { display: flex; gap: 12px; align-items: flex-start; }
      .grow { flex: 1; display: flex; flex-direction: column; gap: 8px; }
      .row { display: flex; gap: 12px; align-items: flex-end; flex-wrap: wrap; }
      .presets { display: flex; gap: 4px; }
      .swatch { width: 18px; height: 18px; border-radius: 50%; border: 1px solid var(--border-3); padding: 0; cursor: pointer; }
      .swatch[aria-pressed='true'] { outline: 2px solid var(--accent); outline-offset: 1px; }
      .err { color: var(--danger-text); }
      .actions { display: flex; justify-content: flex-end; gap: 8px; }
    `,
  ],
})
export class MarkerOverrideDialog {
  protected readonly ref = inject<DialogRef<OverrideStyle>>(DIALOG_REF);
  readonly row = input.required<PluginKindRow>();
  /** The profile's current override of the kind's style. */
  readonly current = input<OverrideStyle>({});

  protected readonly previewLabel = $localize`:@@markers.override.previewLabel:Marker preview`;
  protected readonly presets = COLOR_PRESETS;
  protected readonly shapes = MARKER_SHAPES;
  protected readonly label = linkedSignal(() => this.current().label ?? '');
  protected readonly description = linkedSignal(() => this.current().description ?? '');
  protected readonly color = linkedSignal(() => this.current().color ?? '');
  protected readonly badge = linkedSignal(() => this.current().badge ?? '');
  protected readonly shape = linkedSignal(() => this.current().shape ?? '');

  protected readonly style = computed<OverrideStyle>(() => ({
    label: this.label().trim() || undefined,
    description: this.description().trim() || undefined,
    color: this.color().trim() || undefined,
    badge: this.badge().trim() || undefined,
    shape: this.shape() || undefined,
  }));
  protected readonly problems = computed(() => styleProblems(this.style()));

  protected readonly preview = computed(() => {
    const info = this.row().info;
    const s = this.style();
    return {
      kind: this.row().kind,
      color: s.color ?? info?.color ?? null,
      icon: info?.icon ?? null,
      badge: s.badge ?? info?.badge ?? null,
      shape: s.shape ?? info?.shape ?? null,
    };
  });

  protected colourLabel(colour: string): string {
    return $localize`:@@markers.override.colourAria:Colour ${colour}:colour:`;
  }

  protected save(): void {
    if (!this.problems().length) this.ref.close(this.style());
  }
}
