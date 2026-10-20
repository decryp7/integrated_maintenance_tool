import { ChangeDetectionStrategy, Component, computed, inject, input, model, signal } from '@angular/core';
import { badgeText } from './glyph-model';
import { MarkerGlyph } from './marker-glyph';
import { searchIcons } from './builtin-icons';
import { IconFilePicker, readIconFile } from './icon-import';
import { MarkerStyles } from './marker-styles';
import { MARKER_SHAPES } from '../core/api.types';
import { GlyphStyle, PendingIcon } from './markers.types';

export type GlyphMode = 'badge' | 'icon' | 'custom';
const MODES: { id: GlyphMode; label: string }[] = [
  { id: 'badge', label: $localize`:@@markers.glyphPicker.mode.badge:Badge` },
  { id: 'icon', label: $localize`:@@markers.glyphPicker.mode.icon:Icon` },
  { id: 'custom', label: $localize`:@@markers.glyphPicker.mode.custom:Custom` },
];

/** Chooses how a marker looks: a generated badge, a built-in icon or imported artwork. */
@Component({
  selector: 'imt-glyph-picker',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [MarkerGlyph],
  template: `
    <div class="tabs" role="tablist" aria-label="Marker look" i18n-aria-label="@@markers.glyphPicker.look">
      @for (m of modes; track m.id) {
        <button type="button" role="tab" class="tab" [id]="'gp-tab-' + m.id" [attr.aria-selected]="mode() === m.id" [attr.aria-controls]="'gp-panel-' + m.id" [tabindex]="mode() === m.id ? 0 : -1" (click)="mode.set(m.id)" (keydown)="tabKey($event)">{{ m.label }}</button>
      }
    </div>
    <div class="panel" role="tabpanel" [id]="'gp-panel-' + mode()" [attr.aria-labelledby]="'gp-tab-' + mode()">
      @switch (mode()) {
        @case ('badge') {
          <div class="row">
            <label class="lbl"><ng-container i18n="@@markers.glyphPicker.badgeText">Badge text (1 to 3 characters)</ng-container>
              <input class="input mono" maxlength="3" [value]="badge()" (input)="badge.set($any($event.target).value)" />
            </label>
            @if (suggestion()) {
              <button type="button" class="btn" (click)="badge.set(suggestion())" i18n="@@markers.glyphPicker.useNextFree">Use next free: {{ suggestion() }}</button>
            }
          </div>
          <div class="shapes" role="group" aria-label="Badge shape" i18n-aria-label="@@markers.glyphPicker.badgeShape">
            @for (s of shapes; track s) {
              <button type="button" class="pick" [attr.aria-pressed]="shape() === s" [attr.aria-label]="shapeLabel(s)" [title]="s" (click)="shape.set(s)">
                <imt-marker-glyph [glyph]="shapeStyle(s)" [size]="22" />
              </button>
            }
          </div>
        }
        @case ('icon') {
          <label class="lbl"><ng-container i18n="@@markers.glyphPicker.searchIcons">Search icons</ng-container>
            <input class="input" type="search" [value]="query()" placeholder="e.g. database, timeout" i18n-placeholder="@@markers.glyphPicker.searchPlaceholder" (input)="query.set($any($event.target).value)" />
          </label>
          <div class="grid" role="group" aria-label="Built-in icons" i18n-aria-label="@@markers.glyphPicker.builtinIcons">
            @for (i of results(); track i.name) {
              <button type="button" class="pick" [attr.aria-pressed]="icon() === i.name" [attr.aria-label]="i.name" [title]="i.name" (click)="icon.set(i.name)">
                <imt-marker-glyph [glyph]="iconStyle(i.name)" [size]="20" />
              </button>
            } @empty {
              <span class="faint" i18n="@@markers.glyphPicker.noMatch">No icon matches "{{ query() }}".</span>
            }
          </div>
        }
        @case ('custom') {
          <div class="row">
            <button type="button" class="btn" (click)="importFile()" i18n="@@markers.glyphPicker.import">Import SVG or PNG…</button>
            <span class="faint" i18n="@@markers.glyphPicker.importHint">Up to 16 KB. The engine sanitises SVG and checks PNG when you save.</span>
          </div>
          @if (importError(); as e) { <div class="err" role="alert">{{ e }}</div> }
          @if (pending(); as p) {
            <div class="current"><imt-marker-glyph [glyph]="pendingStyle(p)" [size]="28" /> <span i18n="@@markers.glyphPicker.newIcon">New icon <span class="mono">{{ p.name }}</span> (saved with the marker)</span></div>
          }
          @if (existing().length) {
            <div class="grid" role="group" aria-label="Custom icons in this profile" i18n-aria-label="@@markers.glyphPicker.customIcons">
              @for (i of existing(); track i.name) {
                <button type="button" class="pick" [attr.aria-pressed]="!pending() && icon() === i.name" [attr.aria-label]="i.name" [title]="i.name" (click)="useExisting(i.name)">
                  <imt-marker-glyph [glyph]="iconStyle(i.name)" [size]="20" />
                </button>
              }
            </div>
          }
        }
      }
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 8px; }
    .tabs { display: flex; gap: 2px; border-bottom: 1px solid var(--border); }
    .tab { border: 0; background: transparent; color: var(--muted); padding: 5px 12px; cursor: pointer; border-bottom: 2px solid transparent; }
    .tab[aria-selected='true'] { color: var(--text); border-bottom-color: var(--accent); }
    .panel { display: flex; flex-direction: column; gap: 8px; min-height: 96px; }
    .row { display: flex; gap: 12px; align-items: flex-end; flex-wrap: wrap; }
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--muted); }
    .shapes, .grid { display: flex; flex-wrap: wrap; gap: 4px; max-height: 150px; overflow: auto; }
    .pick { width: 34px; height: 34px; display: inline-flex; align-items: center; justify-content: center; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-2); color: var(--text-2); cursor: pointer; }
    .pick[aria-pressed='true'] { border-color: var(--accent); background: var(--accent-soft); }
    .current { display: flex; align-items: center; gap: 8px; }
    .err { color: var(--danger-text); font-size: var(--fs-sm); }
  `,
})
export class GlyphPicker {
  private readonly picker = inject(IconFilePicker);
  private readonly styles = inject(MarkerStyles);

  readonly mode = model<GlyphMode>('badge');
  readonly badge = model('');
  readonly shape = model('circle');
  readonly icon = model('');
  readonly pending = model<PendingIcon | null>(null);
  readonly color = input<string | null>(null);
  readonly suggestion = input('');
  /** Names the new icon must not reuse (icons of the profile). */
  readonly takenNames = input<ReadonlySet<string>>(new Set());

  protected readonly modes = MODES;
  protected readonly shapes = MARKER_SHAPES;
  protected readonly query = signal('');
  protected readonly importError = signal<string | null>(null);
  /** Text the shape buttons show (their accessible names include it). */
  protected readonly sample = computed(() => badgeText(this.badge()) ?? '1');
  protected readonly results = computed(() => searchIcons(this.query()));
  protected readonly existing = computed(() => this.styles.icons());

  protected shapeLabel(shape: string): string {
    return $localize`:@@markers.glyphPicker.shapeAria:${shape}:shape: badge ${this.sample()}:text:`;
  }
  protected shapeStyle(shape: string): GlyphStyle {
    return { color: this.color(), badge: this.sample(), shape };
  }
  protected iconStyle(icon: string): GlyphStyle {
    return { color: this.color(), icon };
  }
  protected pendingStyle(p: PendingIcon): GlyphStyle {
    return { color: this.color(), svg: p.svg, pngDataUrl: p.pngDataUrl };
  }

  protected useExisting(name: string): void {
    this.pending.set(null);
    this.icon.set(name);
  }

  protected tabKey(e: KeyboardEvent): void {
    const i = MODES.findIndex((m) => m.id === this.mode());
    const next = e.key === 'ArrowRight' ? i + 1 : e.key === 'ArrowLeft' ? i - 1 : null;
    if (next === null) return;
    e.preventDefault();
    const id = MODES[(next + MODES.length) % MODES.length]?.id;
    if (id === undefined) return;
    this.mode.set(id);
    queueMicrotask(() => document.getElementById('gp-tab-' + id)?.focus());
  }

  protected async importFile(): Promise<void> {
    this.importError.set(null);
    const file = await this.picker.pick();
    if (!file) return;
    const taken = new Set([...this.takenNames(), ...this.existing().map((i) => i.name)]);
    const res = await readIconFile(file, taken);
    if (!res.ok) {
      this.importError.set(res.error);
      return;
    }
    this.pending.set(res.icon);
    this.icon.set(res.icon.name);
  }
}
