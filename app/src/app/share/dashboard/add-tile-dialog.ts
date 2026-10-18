import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { TILE_KINDS, TileCfg, TileKind } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { DIALOG_FORM_STYLES } from '../../context/dialog-styles';
import { DIALOG_REF } from '../../ui/dialog.service';

const KIND_LABELS: Record<TileKind, string> = {
  chart: $localize`:@@share.addTile.kind.chart:Chart`,
  stat: $localize`:@@share.addTile.kind.stat:Number (stats query)`,
  table: $localize`:@@share.addTile.kind.table:Table (query)`,
  top: $localize`:@@share.addTile.kind.top:Top values of a field`,
  alerts: $localize`:@@share.addTile.kind.alerts:Alerts`,
  annotations: $localize`:@@share.addTile.kind.annotations:Annotations`,
};

/** Default size per kind, in grid units. */
const SIZES: Record<TileKind, [number, number]> = { chart: [6, 4], stat: [3, 2], table: [6, 4], top: [4, 4], alerts: [4, 3], annotations: [4, 3] };

export interface TileForm {
  type: TileKind;
  title: string;
  chart: string;
  value: string;
  by: string;
  style: 'line' | 'bar' | 'stacked';
  query: string;
  unit: string;
  field: string;
  limit: string;
  window: string;
}

/** The tile a form describes, or the first thing missing (the engine checks the rest on save). */
export function tileFromForm(f: TileForm): { tile: TileCfg } | { error: string } {
  const [w, h] = SIZES[f.type];
  const tile: TileCfg = { id: f.title || f.type, title: f.title.trim(), type: f.type, x: 0, y: 0, w, h };
  const query = f.query.trim();
  const limit = f.limit.trim() ? Number(f.limit) : undefined;
  if (limit !== undefined && (!Number.isInteger(limit) || limit < 1)) return { error: $localize`:@@share.addTile.err.limit:Limit must be a whole number of at least 1.` };
  switch (f.type) {
    case 'chart':
      if (f.chart) tile.chart = f.chart;
      else if (f.value.trim()) {
        tile.value = f.value.trim();
        const by = f.by.split(',').map((s) => s.trim()).filter(Boolean);
        if (by.length) tile.by = by;
        tile.style = f.style;
      } else return { error: $localize`:@@share.addTile.err.chart:Choose a chart, or enter a value such as count() or avg(cpu).` };
      if (query) tile.query = query;
      break;
    case 'stat':
      if (!/\|\s*stats\b/i.test(query)) return { error: $localize`:@@share.addTile.err.stat:A number needs a stats query, e.g. level >= error | stats count()` };
      tile.query = query;
      if (f.unit.trim()) tile.unit = f.unit.trim();
      break;
    case 'table':
      if (!query) return { error: $localize`:@@share.addTile.err.table:A table needs a query.` };
      tile.query = query;
      if (limit) tile.limit = limit;
      break;
    case 'top':
      if (!f.field.trim()) return { error: $localize`:@@share.addTile.err.field:Choose a field.` };
      tile.field = f.field.trim();
      if (query) tile.query = query;
      if (limit) tile.limit = limit;
      break;
    case 'alerts':
      if (query) tile.query = query;
      if (limit) tile.limit = limit;
      break;
    case 'annotations':
      if (limit) tile.limit = limit;
      break;
  }
  if (f.window.trim()) tile.window = f.window.trim();
  return { tile };
}

/** "Add tile": picks the kind and its settings; closes with the new tile. */
@Component({
  selector: 'imt-add-tile-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <form (submit)="$event.preventDefault(); submit()">
      <div class="row">
        <label class="lbl"><ng-container i18n="@@share.addTile.kind">Kind</ng-container>
          <select class="select" name="type" [value]="form().type" (change)="set('type', $any($event.target).value)">
            @for (k of kinds; track k) {<option [value]="k">{{ labels[k] }}</option>}
          </select>
        </label>
        <label class="lbl"><ng-container i18n="@@share.addTile.title">Title</ng-container><input class="input" name="title" [value]="form().title" (input)="set('title', $any($event.target).value)" /></label>
      </div>
      @switch (form().type) {
        @case ('chart') {
          <div class="row">
            <label class="lbl"><ng-container i18n="@@share.addTile.profileChart">Profile chart</ng-container>
              <select class="select" name="chart" [value]="form().chart" (change)="set('chart', $any($event.target).value)">
                <option value="" i18n="@@share.addTile.inlineChart">Inline chart…</option>
                @for (c of charts(); track c.id) {<option [value]="c.id">{{ c.title }}</option>}
              </select>
            </label>
          </div>
          @if (!form().chart) {
            <div class="row">
              <label class="lbl"><ng-container i18n="@@share.addTile.value">Value</ng-container><input class="input mono" name="value" placeholder="count()" [value]="form().value" (input)="set('value', $any($event.target).value)" /></label>
              <label class="lbl"><ng-container i18n="@@share.addTile.splitBy">Split by (comma separated)</ng-container><input class="input mono" name="by" [value]="form().by" (input)="set('by', $any($event.target).value)" /></label>
              <label class="lbl"><ng-container i18n="@@share.addTile.style">Style</ng-container>
                <select class="select" name="style" [value]="form().style" (change)="set('style', $any($event.target).value)">
                  <option value="line" i18n="@@share.addTile.styleLine">Line</option><option value="bar" i18n="@@share.addTile.styleBar">Bar</option><option value="stacked" i18n="@@share.addTile.styleStacked">Stacked</option>
                </select>
              </label>
            </div>
          }
          <label class="lbl"><ng-container i18n="@@share.addTile.filterQl">Filter (query language, optional)</ng-container><input class="input mono" name="query" [value]="form().query" (input)="set('query', $any($event.target).value)" /></label>
        }
        @case ('stat') {
          <label class="lbl"><ng-container i18n="@@share.addTile.statsQuery">Stats query</ng-container><input class="input mono" name="query" placeholder="level >= error | stats count()" [value]="form().query" (input)="set('query', $any($event.target).value)" /></label>
          <label class="lbl"><ng-container i18n="@@share.addTile.unit">Unit (optional)</ng-container><input class="input" name="unit" [value]="form().unit" (input)="set('unit', $any($event.target).value)" /></label>
        }
        @case ('table') {
          <label class="lbl"><ng-container i18n="@@share.addTile.query">Query</ng-container><input class="input mono" name="query" placeholder="| stats count() by SourceContext | sort count() desc | head 10" [value]="form().query" (input)="set('query', $any($event.target).value)" /></label>
          <label class="lbl"><ng-container i18n="@@share.addTile.rows">Rows (optional)</ng-container><input class="input" name="limit" inputmode="numeric" [value]="form().limit" (input)="set('limit', $any($event.target).value)" /></label>
        }
        @case ('top') {
          <div class="row">
            <label class="lbl"><ng-container i18n="@@share.addTile.field">Field</ng-container>
              <select class="select" name="field" [value]="form().field" (change)="set('field', $any($event.target).value)">
                <option value="" i18n="@@share.addTile.choose">Choose…</option>
                @for (f of fields(); track f) {<option [value]="f">{{ f }}</option>}
              </select>
            </label>
            <label class="lbl"><ng-container i18n="@@share.addTile.values">Values (optional)</ng-container><input class="input" name="limit" inputmode="numeric" [value]="form().limit" (input)="set('limit', $any($event.target).value)" /></label>
          </div>
          <label class="lbl"><ng-container i18n="@@share.addTile.filterOptional">Filter (optional)</ng-container><input class="input mono" name="query" [value]="form().query" (input)="set('query', $any($event.target).value)" /></label>
        }
        @case ('alerts') {
          <label class="lbl"><ng-container i18n="@@share.addTile.filterOptional">Filter (optional)</ng-container><input class="input mono" name="query" [value]="form().query" (input)="set('query', $any($event.target).value)" /></label>
          <label class="lbl"><ng-container i18n="@@share.addTile.mostAlerts">Most alerts (optional)</ng-container><input class="input" name="limit" inputmode="numeric" [value]="form().limit" (input)="set('limit', $any($event.target).value)" /></label>
        }
        @case ('annotations') {
          <label class="lbl"><ng-container i18n="@@share.addTile.mostAnnotations">Most annotations (optional)</ng-container><input class="input" name="limit" inputmode="numeric" [value]="form().limit" (input)="set('limit', $any($event.target).value)" /></label>
        }
      }
      <label class="lbl"><ng-container i18n="@@share.addTile.window">Own sliding window (optional, e.g. 15m)</ng-container><input class="input mono" name="window" [value]="form().window" (input)="set('window', $any($event.target).value)" /></label>
      @if (error(); as e) {<p class="err" role="alert">{{ e }}</p>}
      <div class="foot">
        <button type="button" class="btn" (click)="ref.close()" i18n="@@share.addTile.cancel">Cancel</button>
        <button type="submit" class="btn primary" i18n="@@share.addTile.add">Add tile</button>
      </div>
    </form>
  `,
  styles: [DIALOG_FORM_STYLES, `form { display: flex; flex-direction: column; gap: 12px; }`],
})
export class AddTileDialog {
  protected readonly ref = inject(DIALOG_REF);
  private readonly store = inject(WorkspaceStore);

  protected readonly kinds = TILE_KINDS;
  protected readonly labels = KIND_LABELS;
  protected readonly charts = computed(() => this.store.summary()?.charts ?? []);
  protected readonly fields = computed(() => this.store.fieldNames());
  protected readonly form = signal<TileForm>({ type: 'stat', title: '', chart: '', value: '', by: '', style: 'line', query: '', unit: '', field: '', limit: '', window: '' });
  protected readonly error = signal<string | null>(null);

  set<K extends keyof TileForm>(key: K, value: TileForm[K]): void {
    this.form.update((f) => ({ ...f, [key]: value }));
    this.error.set(null);
  }

  submit(): void {
    const out = tileFromForm(this.form());
    if ('error' in out) this.error.set(out.error);
    else this.ref.close(out.tile);
  }
}
