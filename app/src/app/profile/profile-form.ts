import { CommandRegistry } from '../extensions/command-registry';
import { CdkDrag, CdkDragDrop, CdkDragHandle, CdkDropList, moveItemInArray } from '@angular/cdk/drag-drop';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { ChartRule, Condition, MarkerRule, ProfileModel } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { MarkerGlyph } from '../markers/marker-glyph';
import { GlyphStyle } from '../markers/markers.types';
import { HelpHint } from '../ui/help-hint';
import { ConditionsEditor } from './conditions-editor';
import { ExtractorEditor } from './extractor-editor';
import { FlowOverview } from './flow-overview';
import { PipelineEditor } from './pipeline-editor';
import { ProfileDraft } from './profile-draft';
import { StringList } from './string-list';
import { SettingsDialogService } from '../settings/settings-dialog';
import { MANAGE_PARSERS_COMMAND } from '../settings/parsers/parsers.types';

type Section = 'general' | 'files' | 'instance' | 'extract' | 'pipeline' | 'marker' | 'chart';
type Obj = Record<string, unknown>;

/** Returns m[key] as an object, creating it when missing. */
function ensure(m: Obj, key: string): Obj {
  if (!m[key] || typeof m[key] !== 'object') m[key] = {};
  return m[key] as Obj;
}

/** Sets or deletes (for undefined / empty arrays) so the TOML stays tidy. */
function put(o: Obj, key: string, value: unknown): void {
  if (value === undefined || (Array.isArray(value) && value.length === 0 && key !== 'key')) delete o[key];
  else o[key] = value;
}

const AGGS = ['count', 'avg', 'min', 'max', 'sum', 'last'];

@Component({
  selector: 'imt-profile-form',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [StringList, ConditionsEditor, ExtractorEditor, FlowOverview, PipelineEditor, HelpHint, Icon, MarkerGlyph, CdkDropList, CdkDrag, CdkDragHandle],
  template: `
    <datalist id="imt-profile-fields">
      @for (f of fieldChoices(); track f) { <option [value]="f"></option> }
    </datalist>

    <imt-flow-overview (open)="section.set($event)" />
    <div class="main">
    <nav class="nav" aria-label="Profile sections" i18n-aria-label="@@profile.form.sections">
      @for (s of sections; track s.id) {
        <button class="nav-item" [class.on]="section() === s.id" (click)="section.set(s.id)">
          <span>{{ s.label }}</span>
          @if (s.count) { <span class="badge">{{ s.count() }}</span> }
        </button>
      }
    </nav>

    <div class="body">
      @switch (section()) {
        @case ('general') {
          <h2 i18n="@@profile.form.general.title">General</h2>
          <label class="lbl"><ng-container i18n="@@profile.form.general.name">Profile name</ng-container><input class="input" [value]="m().name ?? ''" (input)="set([], 'name', $any($event.target).value)" /></label>
          <p class="help" i18n="@@profile.form.general.help">A profile is a pipeline: which files to read, what identifies an instance, which values to extract, which events are important, and which charts to draw. Extracted values become fields on the event, usable like any Serilog property.</p>
        }
        @case ('files') {
          <h2><ng-container i18n="@@profile.form.files.title">Files</ng-container> <imt-help label="Files" i18n-label="@@profile.form.files.helpLabel" [text]="helpText.files" /></h2>
          <p class="help" i18n="@@profile.form.files.help">Globs matched against the path relative to the opened folder. Extensions do not matter: each file is read by the parser that recognises it.</p>
          <p class="help" data-testid="profile-parsers-hint"><ng-container i18n="@@profile.form.files.parsersHint">Formats are parsers:</ng-container>&ngsp;<button type="button" class="link" (click)="openParsers()" i18n="@@profile.form.files.parsersLink">Settings ▸ Parsers</button></p>
          <div class="lbl" i18n="@@profile.form.files.include">Include</div>
          <imt-string-list label="glob" i18n-label="@@profile.form.glob" [values]="m().files?.include ?? []" (changed)="set(['files'], 'include', $event)" />
          <div class="lbl" i18n="@@profile.form.files.exclude">Exclude</div>
          <imt-string-list label="glob" i18n-label="@@profile.form.glob" [values]="m().files?.exclude ?? []" (changed)="set(['files'], 'exclude', $event)" />
        }
        @case ('instance') {
          <h2><ng-container i18n="@@profile.form.instance.title">Instance</ng-container> <imt-help label="Instance" i18n-label="@@profile.form.instance.helpLabel" [text]="helpText.instance" /></h2>
          <p class="help" i18n="@@profile.form.instance.help">Fields forming the instance key — one timeline lane per distinct combination. Leave the key empty to pick candidates automatically (present on at least half of the events and not constant).</p>
          <div class="grid2">
            <div>
              <div class="lbl" i18n="@@profile.form.instance.key">Key</div>
              <imt-string-list label="field" i18n-label="@@profile.form.field" datalist="imt-profile-fields" [values]="m().instance?.key ?? []" (changed)="set(['instance'], 'key', $event)" />
            </div>
            <div>
              <div class="lbl" i18n="@@profile.form.instance.candidates">Candidates</div>
              <imt-string-list label="field" i18n-label="@@profile.form.field" datalist="imt-profile-fields" [values]="m().instance?.candidates ?? []" (changed)="set(['instance'], 'candidates', $event)" />
            </div>
          </div>
        }
        @case ('extract') {
          <h2><ng-container i18n="@@profile.form.extract.title">Field extractors</ng-container> <imt-help label="Field extractors" i18n-label="@@profile.form.extract.helpLabel" [text]="helpText.extract" /></h2>
          <div class="split">
            <div class="list">
              @for (x of m().extract ?? []; track $index; let i = $index) {
                <div class="list-item" [class.on]="selExtract() === i">
                  <button class="li-main" (click)="selExtract.set(i)">
                    <span class="mono ellipsis">{{ x.name || unnamed }}</span>
                    <span class="faint small ellipsis">{{ outputNames(x.fields) }}</span>
                  </button>
                  <button class="icon-btn" [attr.aria-label]="deleteExtractorLabel(x.name)" (click)="removeAt('extract', i)" [attr.title]="deleteExtractorLabel(x.name)"><imt-icon name="trash" [size]="13" /></button>
                </div>
              }
              <button class="btn dashed" (click)="addExtractor()" i18n="@@profile.form.extract.new">+ New extractor</button>
            </div>
            <div class="detail">
              @if ((m().extract ?? []).length) {
                <imt-extractor-editor [index]="selExtractClamped()" />
              } @else {
                <p class="help" i18n="@@profile.form.extract.help">Extractors pull values out of text with a regex (named groups) and turn them into typed fields — for example CPU and memory from "Process stats CPU 71.2% · WorkingSet 884 MB". Those fields can then be filtered, grouped and charted.</p>
              }
            </div>
          </div>
        }
        @case ('pipeline') {
          <imt-pipeline-editor />
        }
        @case ('marker') {
          <h2><ng-container i18n="@@profile.form.marker.title">Markers</ng-container> <imt-help label="Markers" i18n-label="@@profile.form.marker.helpLabel" [text]="helpText.marker" /></h2>
          <p class="help" i18n="@@profile.form.marker.help">Important events shown on the timeline. Order matters: the first matching rule is the event's primary marker. Drag to reorder.</p>
          <div cdkDropList class="markers" (cdkDropListDropped)="dropMarker($event)">
            @for (mk of m().marker ?? []; track $index; let i = $index) {
              <div class="marker-card" cdkDrag>
                <div class="mk-row">
                  <button class="icon-btn handle" cdkDragHandle aria-label="Drag to reorder" i18n-aria-label="@@profile.form.marker.drag" title="Drag to reorder" i18n-title="@@profile.form.marker.dragTitle"><imt-icon name="drag" [size]="14" [stroke]="3" /></button>
                  <imt-marker-glyph [glyph]="glyphOf(mk)" [size]="18" />
                  <input class="input mono kind" [value]="mk.kind" aria-label="Kind" i18n-aria-label="@@profile.form.marker.kind" (input)="setItem('marker', i, 'kind', $any($event.target).value)" />
                  <input class="input" [value]="mk.label ?? ''" placeholder="Label" i18n-placeholder="@@profile.form.marker.labelPlaceholder" aria-label="Label" i18n-aria-label="@@profile.form.marker.label" (input)="setItem('marker', i, 'label', $any($event.target).value || undefined)" />
                  <input type="color" class="color" [value]="mk.color ?? '#8a91a3'" aria-label="Colour" i18n-aria-label="@@profile.form.marker.colour" (input)="setItem('marker', i, 'color', $any($event.target).value)" />
                  <label class="row small" i18n="@@profile.form.marker.lifecycle"><input type="checkbox" [checked]="!!mk.lifecycle" (change)="setItem('marker', i, 'lifecycle', $any($event.target).checked || undefined)" /> lifecycle</label>
                  <button class="icon-btn" [attr.aria-label]="deleteMarkerLabel(mk.kind)" (click)="removeAt('marker', i)" [attr.title]="deleteMarkerLabel(mk.kind)"><imt-icon name="trash" [size]="13" /></button>
                </div>
                <imt-conditions-editor [conditions]="mk.when ?? []" emptyText="Matches every event — add a condition." i18n-emptyText="@@profile.form.marker.empty" (changed)="setItem('marker', i, 'when', $event)" />
              </div>
            }
          </div>
          <button class="btn dashed" (click)="addMarker()" i18n="@@profile.form.marker.new">+ New marker</button>
        }
        @case ('chart') {
          <h2><ng-container i18n="@@profile.form.chart.title">Charts</ng-container> <imt-help label="Charts" i18n-label="@@profile.form.chart.helpLabel" [text]="helpText.chart" /></h2>
          <p class="help" i18n="@@profile.form.chart.help">Charts shown on the Metrics tab. y is an aggregate of a numeric field per time bucket, optionally split into one series per value of other fields.</p>
          @for (c of m().chart ?? []; track $index; let i = $index) {
            <div class="chart-card">
              <div class="grid4">
                <label class="lbl"><ng-container i18n="@@profile.form.chart.id">Id</ng-container><input class="input mono" [value]="c.id" (input)="setItem('chart', i, 'id', $any($event.target).value)" /></label>
                <label class="lbl"><ng-container i18n="@@profile.form.chart.titleField">Title</ng-container><input class="input" [value]="c.title ?? ''" (input)="setItem('chart', i, 'title', $any($event.target).value || undefined)" /></label>
                <label class="lbl"><ng-container i18n="@@profile.form.chart.aggregate">Aggregate</ng-container>
                  <select class="select" [value]="yAgg(c)" (change)="setY(i, $any($event.target).value, yField(c))">
                    @for (a of aggs; track a) { <option [value]="a" [selected]="a === yAgg(c)">{{ a }}</option> }
                  </select>
                </label>
                <label class="lbl"><ng-container i18n="@@profile.form.chart.field">Field</ng-container>
                  <input class="input mono" list="imt-profile-fields" [value]="yField(c)" [disabled]="yAgg(c) === 'count'" (input)="setY(i, yAgg(c), $any($event.target).value)" />
                </label>
                <label class="lbl"><ng-container i18n="@@profile.form.chart.bucket">Bucket</ng-container><input class="input mono" [value]="c.bucket ?? 'auto'" placeholder="auto, 30s, 1m" (input)="setItem('chart', i, 'bucket', $any($event.target).value || undefined)" /></label>
                <label class="lbl"><ng-container i18n="@@profile.form.chart.type">Type</ng-container>
                  <select class="select" [value]="c.type ?? 'line'" (change)="setItem('chart', i, 'type', $any($event.target).value)">
                    <option value="line">line</option><option value="bar">bar</option><option value="stacked">stacked</option>
                  </select>
                </label>
                <label class="lbl"><ng-container i18n="@@profile.form.chart.unit">Unit</ng-container><input class="input" [value]="c.unit ?? ''" (input)="setItem('chart', i, 'unit', $any($event.target).value || undefined)" /></label>
                <div class="del"><button class="btn ghost" (click)="removeAt('chart', i)" i18n="@@profile.form.chart.delete"><imt-icon name="trash" [size]="13" /> Delete</button></div>
              </div>
              <div class="grid2">
                <div>
                  <div class="lbl" i18n="@@profile.form.chart.splitBy">Split by</div>
                  <imt-string-list label="field" i18n-label="@@profile.form.field" datalist="imt-profile-fields" [values]="c.split_by ?? []" (changed)="setItem('chart', i, 'split_by', $event)" />
                </div>
                <div>
                  <div class="lbl" i18n="@@profile.form.chart.onlyWhen">Only when</div>
                  <imt-conditions-editor [conditions]="c.when ?? []" (changed)="setItem('chart', i, 'when', $event)" />
                </div>
              </div>
            </div>
          }
          <button class="btn dashed" (click)="addChart()" i18n="@@profile.form.chart.new">+ New chart</button>
        }
      }
    </div>
    </div>
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; }
    .main { flex: 1; min-height: 0; display: flex; }
    h2 { display: flex; align-items: center; gap: 8px; }
    .nav { width: 190px; flex: none; padding: 10px 8px; border-right: 1px solid var(--border); display: flex; flex-direction: column; gap: 2px; background: var(--bg-2); overflow-y: auto; }
    /* flex: none keeps every item at its 30px height (a 24px target, WCAG 2.5.8) when the window is short; the list scrolls. */
    .nav-item { flex: none; display: flex; justify-content: space-between; align-items: center; height: 30px; padding: 0 10px; border: 0; border-radius: 6px; background: transparent; color: var(--text-2); cursor: pointer; text-align: left; }
    .nav-item:hover { background: var(--bg-hover); }
    .nav-item.on { background: var(--bg-sel); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
    .body { flex: 1; min-width: 0; overflow: auto; padding: 16px 20px 40px; display: flex; flex-direction: column; gap: 10px; max-width: 1100px; }
    h2 { margin: 0 0 4px; font-size: var(--fs-lg); font-weight: 600; }
    .help { margin: 0; color: var(--muted); line-height: 1.5; max-width: 760px; }
    .link { border: 0; background: none; padding: 0; color: var(--text); cursor: pointer; text-decoration: underline; font: inherit; }
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-sm); color: var(--muted); }
    .lbl.row, .row { flex-direction: row; align-items: center; gap: 6px; display: flex; }
    .narrow { max-width: 240px; }
    .small { font-size: var(--fs-xs); }
    .grid2 { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
    .grid3 { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; align-items: end; }
    .grid4 { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 10px; align-items: end; }
    .split { display: grid; grid-template-columns: 240px minmax(0, 1fr); gap: 16px; }
    .list { display: flex; flex-direction: column; gap: 2px; }
    .list-item { display: flex; align-items: center; border-radius: 6px; }
    .list-item.on { background: var(--bg-sel); box-shadow: inset 2px 0 0 var(--accent); }
    .li-main { flex: 1; min-width: 0; display: flex; flex-direction: column; align-items: flex-start; padding: 6px 8px; border: 0; background: transparent; cursor: pointer; text-align: left; color: var(--text); }
    .detail { min-width: 0; }
    .markers { display: flex; flex-direction: column; gap: 8px; }
    .marker-card, .chart-card { padding: 10px; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); display: flex; flex-direction: column; gap: 8px; }
    .mk-row { display: flex; align-items: center; gap: 8px; }
    .kind { width: 140px; }
    .color { width: 32px; height: 26px; padding: 0; border: 1px solid var(--border-2); border-radius: 4px; background: transparent; }
    .handle { cursor: grab; }
    .cdk-drag-preview { box-shadow: var(--shadow); }
    .cdk-drag-placeholder { opacity: 0.3; }
    .del { display: flex; justify-content: flex-end; }
  `,
})
export class ProfileForm {
  protected readonly draft = inject(ProfileDraft);
  private readonly store = inject(WorkspaceStore);
  private readonly commands = inject(CommandRegistry);
  private readonly settings = inject(SettingsDialogService);
  protected readonly m = this.draft.model;
  protected readonly section = signal<Section>('general');
  protected readonly selExtract = signal(0);
  protected readonly selExtractClamped = computed(() => Math.min(this.selExtract(), Math.max(0, (this.m().extract?.length ?? 1) - 1)));

  protected readonly helpText = {
    files: $localize`:@@profile.form.files.helpText:Which files in an opened folder IMT reads. Include and exclude are glob patterns; anything that is not a log is skipped.`,
    instance: $localize`:@@profile.form.instance.helpText:An instance is one running copy of a service. The key fields decide which events share a timeline lane, for example MachineName + ProcessId.`,
    extract: $localize`:@@profile.form.extract.helpText:Extractors read a value out of the message text (for example CPU %) and store it as a typed field you can filter and chart.`,
    marker: $localize`:@@profile.form.marker.helpText:Markers highlight important events (start, crash, deployment) on the timeline. The first matching rule is the primary marker.`,
    chart: $localize`:@@profile.form.chart.helpText:Charts on the Metrics panel: an aggregate of a numeric field per time bucket, optionally one line per value of other fields.`,
  };
  protected readonly unnamed = $localize`:@@profile.form.extract.unnamed:(unnamed)`;
  protected readonly aggs = AGGS;

  protected readonly sections: { id: Section; label: string; count?: () => number }[] = [
    { id: 'general', label: $localize`:Profile editor section|@@profile.form.nav.general:General` },
    { id: 'files', label: $localize`:Profile editor section|@@profile.form.nav.files:Files` },
    { id: 'instance', label: $localize`:Profile editor section|@@profile.form.nav.instance:Instance` },
    { id: 'extract', label: $localize`:Profile editor section|@@profile.form.nav.extract:Extractors`, count: () => this.m().extract?.length ?? 0 },
    { id: 'pipeline', label: $localize`:Profile editor section|@@profile.form.nav.pipeline:Pipeline`, count: () => this.m().pipeline?.length ?? 0 },
    { id: 'marker', label: $localize`:Profile editor section|@@profile.form.nav.marker:Markers`, count: () => this.m().marker?.length ?? 0 },
    { id: 'chart', label: $localize`:Profile editor section|@@profile.form.nav.chart:Charts`, count: () => this.m().chart?.length ?? 0 },
  ];

  protected readonly fieldChoices = computed(() => ['@level', '@message', '@exception', '@template', '@source', '@line', ...this.store.fieldNames()]);

  protected deleteExtractorLabel(name: string): string {
    return $localize`:@@profile.form.extract.deleteAria:Delete extractor ${name}:name:`;
  }
  protected deleteMarkerLabel(kind: string): string {
    return $localize`:@@profile.form.marker.deleteAria:Delete marker ${kind}:kind:`;
  }
  protected outputNames(fields: Record<string, unknown> | undefined): string {
    return Object.keys(fields ?? {}).join(', ') || $localize`:@@profile.form.extract.noOutputs:no outputs`;
  }
  protected yAgg(c: ChartRule): string {
    return /^(\w+)\(/.exec(c.y)?.[1] ?? 'count';
  }
  protected yField(c: ChartRule): string {
    return /\(([^)]*)\)/.exec(c.y)?.[1] ?? '';
  }

  /** Formats are parsers (ADR 0026): the Parsers view. */
  protected openParsers(): void {
    void this.commands.run(MANAGE_PARSERS_COMMAND);
  }

  /** Sets path.key, creating intermediate tables. */
  protected set(path: string[], key: string, value: unknown): void {
    this.draft.update((m) => {
      let o = m as Obj;
      for (const p of path) o = ensure(o, p);
      put(o, key, value);
    });
  }

  /** How the rule looks (badge, icon or default shape), for the card header. */
  protected glyphOf(mk: MarkerRule): GlyphStyle {
    return { kind: mk.kind, color: mk.color ?? null, icon: mk['icon'] as string | undefined, badge: mk['badge'] as string | undefined, shape: mk['shape'] as string | undefined };
  }

  protected setItem(list: 'marker' | 'chart', i: number, key: string, value: unknown): void {
    this.draft.update((m) => {
      const item = (m[list] as Obj[] | undefined)?.[i];
      if (item) put(item, key, value);
    });
  }

  protected setY(i: number, agg: string, field: string): void {
    this.setItem('chart', i, 'y', agg === 'count' ? 'count()' : `${agg}(${field})`);
  }

  protected removeAt(list: 'extract' | 'marker' | 'chart', i: number): void {
    this.draft.update((m: ProfileModel) => {
      const arr = m[list] as unknown[] | undefined;
      arr?.splice(i, 1);
      if (arr && !arr.length) delete m[list];
    });
  }

  protected addExtractor(): void {
    const n = (this.m().extract?.length ?? 0) + 1;
    this.draft.update((m) => {
      (m.extract ??= []).push({ name: `extractor-${n}`, from: '@message', regex: 'value (?P<value>[\\d.]+)', fields: { value: { type: 'number' } } });
    });
    this.selExtract.set(n - 1);
  }

  protected addMarker(): void {
    const when: Condition[] = [{ field: '@message', op: 'contains', value: '' }];
    this.draft.update((m) => {
      (m.marker ??= []).push({ kind: 'custom', label: $localize`:Default label of a new marker rule|@@profile.form.marker.defaultLabel:Custom`, color: '#9085e9', when } as MarkerRule);
    });
  }

  protected addChart(): void {
    this.draft.update((m) => {
      const n = (m.chart?.length ?? 0) + 1;
      (m.chart ??= []).push({ id: `chart-${n}`, title: $localize`:Default title of a new chart|@@profile.form.chart.defaultTitle:Chart ${n}:number:`, y: 'count()', bucket: 'auto', split_by: [], type: 'line' });
    });
  }

  protected dropMarker(e: CdkDragDrop<unknown>): void {
    if (e.previousIndex === e.currentIndex) return;
    this.draft.update((m) => {
      if (m.marker) moveItemInArray(m.marker, e.previousIndex, e.currentIndex);
    });
  }
}
