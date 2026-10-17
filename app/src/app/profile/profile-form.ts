import { CdkDrag, CdkDragDrop, CdkDragHandle, CdkDropList, moveItemInArray } from '@angular/cdk/drag-drop';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { LEVELS } from '../core/api.types';
import { ChartRule, Condition, MarkerRule, ProfileModel } from '../core/profile.model';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { MarkerIcon } from '../shared/marker-icon';
import { HelpHint } from '../ui/help-hint';
import { ConditionsEditor } from './conditions-editor';
import { ExtractorEditor } from './extractor-editor';
import { FlowOverview } from './flow-overview';
import { PipelineEditor } from './pipeline-editor';
import { ProfileDraft } from './profile-draft';
import { StringList } from './string-list';

type Section = 'general' | 'files' | 'parse' | 'json' | 'csv' | 'instance' | 'extract' | 'pipeline' | 'marker' | 'chart';
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
  imports: [StringList, ConditionsEditor, ExtractorEditor, FlowOverview, PipelineEditor, HelpHint, Icon, MarkerIcon, CdkDropList, CdkDrag, CdkDragHandle],
  template: `
    <datalist id="imt-profile-fields">
      @for (f of fieldChoices(); track f) { <option [value]="f"></option> }
    </datalist>

    <imt-flow-overview (open)="section.set($event)" />
    <div class="main">
    <nav class="nav" aria-label="Profile sections">
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
          <h2>General</h2>
          <label class="lbl">Profile name<input class="input" [value]="m().name ?? ''" (input)="set([], 'name', $any($event.target).value)" /></label>
          <p class="help">A profile is a pipeline: which files to read, how to parse them, what identifies an instance, which values to extract, which events are important, and which charts to draw. Extracted values become fields on the event, usable like any Serilog property.</p>
        }
        @case ('files') {
          <h2>Files <imt-help label="Files" [text]="helpText.files" /></h2>
          <p class="help">Globs matched against the path relative to the opened folder. Extensions do not matter: the format is detected from the content.</p>
          <div class="lbl">Include</div>
          <imt-string-list label="glob" [values]="m().files?.include ?? []" (changed)="set(['files'], 'include', $event)" />
          <div class="lbl">Exclude</div>
          <imt-string-list label="glob" [values]="m().files?.exclude ?? []" (changed)="set(['files'], 'exclude', $event)" />
        }
        @case ('parse') {
          <h2>Parsing <imt-help label="Parsing" [text]="helpText.parse" /></h2>
          <div class="grid3">
            <label class="lbl">Format
              <select class="select" [value]="m().parse?.format ?? 'auto'" (change)="set(['parse'], 'format', $any($event.target).value)">
                @for (f of formats; track f) { <option [value]="f" [selected]="f === (m().parse?.format ?? 'auto')">{{ f }}</option> }
              </select>
            </label>
            <label class="lbl">Timezone for timestamps without offset
              <input class="input mono" [value]="m().parse?.assume_timezone ?? 'UTC'" placeholder="UTC, local, +08:00" (input)="set(['parse'], 'assume_timezone', $any($event.target).value)" />
            </label>
            <label class="lbl">Day first (03/04 = 3 April)
              <select class="select" [value]="dayFirst()" (change)="set(['parse'], 'day_first', $any($event.target).value)">
                <option value="auto">auto (per file)</option>
                <option value="true">true</option>
                <option value="false">false</option>
              </select>
            </label>
          </div>
          <div class="lbl">Serilog output templates (plain-text files, tried in order)</div>
          <imt-string-list label="template" [values]="m().parse?.output_templates ?? []" (changed)="set(['parse'], 'output_templates', $event)" />
          <div class="lbl">Regexes (named groups: timestamp, level, message; others become fields)</div>
          <imt-string-list label="regex" [values]="m().parse?.regexes ?? []" (changed)="set(['parse'], 'regexes', $event)" />
          <div class="lbl">Extra timestamp formats (.NET "dd/MM/yyyy HH:mm:ss.fff" or strftime "%d/%m/%Y %H:%M:%S")</div>
          <imt-string-list label="format" [values]="m().parse?.timestamp_formats ?? []" (changed)="set(['parse'], 'timestamp_formats', $event)" />
        }
        @case ('json') {
          <h2>JSON logs</h2>
          <p class="help">JSON lines or a whole-file array. Keys are tried in order; nested keys use dots ("log.level"). Covers Serilog CLEF, JsonFormatter, NLog, ECS and most JSON loggers.</p>
          <div class="grid2">
            @for (k of jsonKeys; track k.key) {
              <div>
                <div class="lbl">{{ k.label }}</div>
                <imt-string-list [label]="'key'" [values]="jsonList(k.key)" (changed)="set(['parse', 'json'], k.key, $event)" />
              </div>
            }
          </div>
          <label class="lbl narrow">Default level (when absent)
            <select class="select" [value]="m().parse?.json?.default_level ?? 'Information'" (change)="set(['parse', 'json'], 'default_level', $any($event.target).value)">
              @for (l of levels; track l) { <option [value]="l" [selected]="l === (m().parse?.json?.default_level ?? 'Information')">{{ l }}</option> }
            </select>
          </label>
        }
        @case ('csv') {
          <h2>CSV / delimited</h2>
          <div class="grid3">
            <label class="lbl">Delimiter
              <select class="select mono" [value]="m().parse?.csv?.delimiter ?? 'auto'" (change)="set(['parse', 'csv'], 'delimiter', $any($event.target).value)">
                <option value="auto">auto</option>
                <option value=",">,</option>
                <option value=";">;</option>
                <option value="\t">tab</option>
                <option value="|">|</option>
              </select>
            </label>
            <label class="lbl row"><input type="checkbox" [checked]="m().parse?.csv?.has_header ?? true" (change)="set(['parse', 'csv'], 'has_header', $any($event.target).checked)" /> First row is a header</label>
          </div>
          <div class="grid3">
            @for (c of csvCols; track c) {
              <label class="lbl">{{ c }} column
                <input class="input mono" [value]="csvCol(c)" placeholder="auto" (input)="set(['parse', 'csv'], c, $any($event.target).value || 'auto')" />
              </label>
            }
          </div>
        }
        @case ('instance') {
          <h2>Instance <imt-help label="Instance" [text]="helpText.instance" /></h2>
          <p class="help">Fields forming the instance key — one timeline lane per distinct combination. Leave the key empty to pick candidates automatically (present on at least half of the events and not constant).</p>
          <div class="grid2">
            <div>
              <div class="lbl">Key</div>
              <imt-string-list label="field" datalist="imt-profile-fields" [values]="m().instance?.key ?? []" (changed)="set(['instance'], 'key', $event)" />
            </div>
            <div>
              <div class="lbl">Candidates</div>
              <imt-string-list label="field" datalist="imt-profile-fields" [values]="m().instance?.candidates ?? []" (changed)="set(['instance'], 'candidates', $event)" />
            </div>
          </div>
        }
        @case ('extract') {
          <h2>Field extractors <imt-help label="Field extractors" [text]="helpText.extract" /></h2>
          <div class="split">
            <div class="list">
              @for (x of m().extract ?? []; track $index; let i = $index) {
                <div class="list-item" [class.on]="selExtract() === i">
                  <button class="li-main" (click)="selExtract.set(i)">
                    <span class="mono ellipsis">{{ x.name || '(unnamed)' }}</span>
                    <span class="faint small ellipsis">{{ outputNames(x.fields) }}</span>
                  </button>
                  <button class="icon-btn" [attr.aria-label]="'Delete extractor ' + x.name" (click)="removeAt('extract', i)" [attr.title]="'Delete extractor ' + x.name"><imt-icon name="trash" [size]="13" /></button>
                </div>
              }
              <button class="btn dashed" (click)="addExtractor()">+ New extractor</button>
            </div>
            <div class="detail">
              @if ((m().extract ?? []).length) {
                <imt-extractor-editor [index]="selExtractClamped()" />
              } @else {
                <p class="help">Extractors pull values out of text with a regex (named groups) and turn them into typed fields — for example CPU and memory from "Process stats CPU 71.2% · WorkingSet 884 MB". Those fields can then be filtered, grouped and charted.</p>
              }
            </div>
          </div>
        }
        @case ('pipeline') {
          <imt-pipeline-editor />
        }
        @case ('marker') {
          <h2>Markers <imt-help label="Markers" [text]="helpText.marker" /></h2>
          <p class="help">Important events shown on the timeline. Order matters: the first matching rule is the event's primary marker. Drag to reorder.</p>
          <div cdkDropList class="markers" (cdkDropListDropped)="dropMarker($event)">
            @for (mk of m().marker ?? []; track $index; let i = $index) {
              <div class="marker-card" cdkDrag>
                <div class="mk-row">
                  <button class="icon-btn handle" cdkDragHandle aria-label="Drag to reorder" title="Drag to reorder"><imt-icon name="drag" [size]="14" [stroke]="3" /></button>
                  <imt-marker-icon [kind]="mk.kind" [color]="mk.color ?? null" />
                  <input class="input mono kind" [value]="mk.kind" aria-label="Kind" (input)="setItem('marker', i, 'kind', $any($event.target).value)" />
                  <input class="input" [value]="mk.label ?? ''" placeholder="Label" aria-label="Label" (input)="setItem('marker', i, 'label', $any($event.target).value || undefined)" />
                  <input type="color" class="color" [value]="mk.color ?? '#8a91a3'" aria-label="Colour" (input)="setItem('marker', i, 'color', $any($event.target).value)" />
                  <label class="row small"><input type="checkbox" [checked]="!!mk.lifecycle" (change)="setItem('marker', i, 'lifecycle', $any($event.target).checked || undefined)" /> lifecycle</label>
                  <button class="icon-btn" [attr.aria-label]="'Delete marker ' + mk.kind" (click)="removeAt('marker', i)" [attr.title]="'Delete marker ' + mk.kind"><imt-icon name="trash" [size]="13" /></button>
                </div>
                <imt-conditions-editor [conditions]="mk.when ?? []" emptyText="Matches every event — add a condition." (changed)="setItem('marker', i, 'when', $event)" />
              </div>
            }
          </div>
          <button class="btn dashed" (click)="addMarker()">+ New marker</button>
        }
        @case ('chart') {
          <h2>Charts <imt-help label="Charts" [text]="helpText.chart" /></h2>
          <p class="help">Charts shown on the Metrics tab. y is an aggregate of a numeric field per time bucket, optionally split into one series per value of other fields.</p>
          @for (c of m().chart ?? []; track $index; let i = $index) {
            <div class="chart-card">
              <div class="grid4">
                <label class="lbl">Id<input class="input mono" [value]="c.id" (input)="setItem('chart', i, 'id', $any($event.target).value)" /></label>
                <label class="lbl">Title<input class="input" [value]="c.title ?? ''" (input)="setItem('chart', i, 'title', $any($event.target).value || undefined)" /></label>
                <label class="lbl">Aggregate
                  <select class="select" [value]="yAgg(c)" (change)="setY(i, $any($event.target).value, yField(c))">
                    @for (a of aggs; track a) { <option [value]="a" [selected]="a === yAgg(c)">{{ a }}</option> }
                  </select>
                </label>
                <label class="lbl">Field
                  <input class="input mono" list="imt-profile-fields" [value]="yField(c)" [disabled]="yAgg(c) === 'count'" (input)="setY(i, yAgg(c), $any($event.target).value)" />
                </label>
                <label class="lbl">Bucket<input class="input mono" [value]="c.bucket ?? 'auto'" placeholder="auto, 30s, 1m" (input)="setItem('chart', i, 'bucket', $any($event.target).value || undefined)" /></label>
                <label class="lbl">Type
                  <select class="select" [value]="c.type ?? 'line'" (change)="setItem('chart', i, 'type', $any($event.target).value)">
                    <option value="line">line</option><option value="bar">bar</option><option value="stacked">stacked</option>
                  </select>
                </label>
                <label class="lbl">Unit<input class="input" [value]="c.unit ?? ''" (input)="setItem('chart', i, 'unit', $any($event.target).value || undefined)" /></label>
                <div class="del"><button class="btn ghost" (click)="removeAt('chart', i)"><imt-icon name="trash" [size]="13" /> Delete</button></div>
              </div>
              <div class="grid2">
                <div>
                  <div class="lbl">Split by</div>
                  <imt-string-list label="field" datalist="imt-profile-fields" [values]="c.split_by ?? []" (changed)="setItem('chart', i, 'split_by', $event)" />
                </div>
                <div>
                  <div class="lbl">Only when</div>
                  <imt-conditions-editor [conditions]="c.when ?? []" (changed)="setItem('chart', i, 'when', $event)" />
                </div>
              </div>
            </div>
          }
          <button class="btn dashed" (click)="addChart()">+ New chart</button>
        }
      }
    </div>
    </div>
  `,
  styles: `
    :host { flex: 1; min-height: 0; display: flex; flex-direction: column; }
    .main { flex: 1; min-height: 0; display: flex; }
    h2 { display: flex; align-items: center; gap: 8px; }
    .nav { width: 190px; flex: none; padding: 10px 8px; border-right: 1px solid var(--border); display: flex; flex-direction: column; gap: 2px; background: var(--bg-2); }
    .nav-item { display: flex; justify-content: space-between; align-items: center; height: 30px; padding: 0 10px; border: 0; border-radius: 6px; background: transparent; color: var(--text-2); cursor: pointer; text-align: left; }
    .nav-item:hover { background: var(--bg-hover); }
    .nav-item.on { background: var(--bg-sel); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
    .body { flex: 1; min-width: 0; overflow: auto; padding: 16px 20px 40px; display: flex; flex-direction: column; gap: 10px; max-width: 1100px; }
    h2 { margin: 0 0 4px; font-size: 16px; font-weight: 600; }
    .help { margin: 0; color: var(--muted); line-height: 1.5; max-width: 760px; }
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: 12px; color: var(--muted); }
    .lbl.row, .row { flex-direction: row; align-items: center; gap: 6px; display: flex; }
    .narrow { max-width: 240px; }
    .small { font-size: 11px; }
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
  protected readonly m = this.draft.model;
  protected readonly section = signal<Section>('general');
  protected readonly selExtract = signal(0);
  protected readonly selExtractClamped = computed(() => Math.min(this.selExtract(), Math.max(0, (this.m().extract?.length ?? 1) - 1)));

  protected readonly helpText = {
    files: 'Which files in an opened folder IMT reads. Include and exclude are glob patterns; anything that is not a log is skipped.',
    parse: 'How a file becomes events: IMT detects the format and tries your templates and regexes in order to find timestamp, level and message.',
    instance: 'An instance is one running copy of a service. The key fields decide which events share a timeline lane, for example MachineName + ProcessId.',
    extract: 'Extractors read a value out of the message text (for example CPU %) and store it as a typed field you can filter and chart.',
    marker: 'Markers highlight important events (start, crash, deployment) on the timeline. The first matching rule is the primary marker.',
    chart: 'Charts on the Metrics panel: an aggregate of a numeric field per time bucket, optionally one line per value of other fields.',
  };
  protected readonly formats = ['auto', 'json', 'text', 'csv'];
  protected readonly levels = LEVELS;
  protected readonly aggs = AGGS;
  protected readonly csvCols = ['timestamp', 'level', 'message'] as const;
  protected readonly jsonKeys = [
    { key: 'timestamp', label: 'Timestamp keys' },
    { key: 'level', label: 'Level keys' },
    { key: 'message', label: 'Message keys' },
    { key: 'template', label: 'Template keys' },
    { key: 'exception', label: 'Exception keys' },
    { key: 'properties', label: 'Property containers' },
  ];

  protected readonly sections: { id: Section; label: string; count?: () => number }[] = [
    { id: 'general', label: 'General' },
    { id: 'files', label: 'Files' },
    { id: 'parse', label: 'Parsing' },
    { id: 'json', label: 'JSON' },
    { id: 'csv', label: 'CSV' },
    { id: 'instance', label: 'Instance' },
    { id: 'extract', label: 'Extractors', count: () => this.m().extract?.length ?? 0 },
    { id: 'pipeline', label: 'Pipeline', count: () => this.m().pipeline?.length ?? 0 },
    { id: 'marker', label: 'Markers', count: () => this.m().marker?.length ?? 0 },
    { id: 'chart', label: 'Charts', count: () => this.m().chart?.length ?? 0 },
  ];

  protected readonly fieldChoices = computed(() => ['@level', '@message', '@exception', '@template', '@source', '@line', ...this.store.fieldNames()]);
  protected readonly dayFirst = computed(() => String(this.m().parse?.day_first ?? 'auto'));

  protected jsonList(key: string): string[] {
    return ((this.m().parse?.json as Obj | undefined)?.[key] as string[] | undefined) ?? [];
  }
  protected csvCol(c: string): string {
    const v = (this.m().parse?.csv as Obj | undefined)?.[c];
    return typeof v === 'string' && v !== 'auto' ? v : '';
  }
  protected outputNames(fields: Record<string, unknown> | undefined): string {
    return Object.keys(fields ?? {}).join(', ') || 'no outputs';
  }
  protected yAgg(c: ChartRule): string {
    return /^(\w+)\(/.exec(c.y)?.[1] ?? 'count';
  }
  protected yField(c: ChartRule): string {
    return /\(([^)]*)\)/.exec(c.y)?.[1] ?? '';
  }

  /** Sets path.key, creating intermediate tables. */
  protected set(path: string[], key: string, value: unknown): void {
    this.draft.update((m) => {
      let o = m as Obj;
      for (const p of path) o = ensure(o, p);
      put(o, key, value);
    });
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
      (m.marker ??= []).push({ kind: 'custom', label: 'Custom', color: '#9085e9', when } as MarkerRule);
    });
  }

  protected addChart(): void {
    this.draft.update((m) => {
      const n = (m.chart?.length ?? 0) + 1;
      (m.chart ??= []).push({ id: `chart-${n}`, title: `Chart ${n}`, y: 'count()', bucket: 'auto', split_by: [], type: 'line' });
    });
  }

  protected dropMarker(e: CdkDragDrop<unknown>): void {
    if (e.previousIndex === e.currentIndex) return;
    this.draft.update((m) => {
      if (m.marker) moveItemInArray(m.marker, e.previousIndex, e.currentIndex);
    });
  }
}
