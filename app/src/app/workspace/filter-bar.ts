import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, signal, untracked, viewChild } from '@angular/core';
import { FilterOp, LEVELS, Level } from '../core/api.types';
import { formatCompact, formatDateTime, formatDuration, tzLabel } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { ClickOutside } from '../shared/click-outside';
import { LEVEL_CODES, LEVEL_VARS } from '../shared/colors';
import { Icon } from '../shared/icon';
import { TimelineZoom } from '../timeline/zoom';

const OP_LABELS: Record<FilterOp, string> = {
  eq: '=',
  ne: '≠',
  contains: '~',
  notContains: '!~',
  matches: 'matches',
  exists: 'exists',
  missing: 'missing',
  gt: '>',
  lt: '<',
};

@Component({
  selector: 'imt-filter-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, ClickOutside],
  template: `
    <div class="row">
      <label class="search" [class.invalid]="regexInvalid()">
        <imt-icon name="search" [size]="14" class="faint" />
        <input
          #search
          class="mono"
          aria-label="Search messages and exceptions"
          [placeholder]="q().regex ? 'Regular expression' : 'Search messages, exceptions, templates'"
          [value]="store.textInput()"
          (input)="store.setTextInput($any($event.target).value)"
          (keydown.escape)="store.setTextInput('')"
        />
        <button class="toggle mono" [class.on]="q().regex" (click)="store.setRegex(!q().regex)" title="Regular expression" [attr.aria-pressed]="q().regex">.*</button>
        <button class="toggle mono" [class.on]="q().caseSensitive" (click)="store.setCaseSensitive(!q().caseSensitive)" title="Match case" [attr.aria-pressed]="q().caseSensitive">Aa</button>
      </label>

      <div class="levels" role="group" aria-label="Levels">
        @for (l of levels; track l) {
          <button class="lvl" [class.on]="levelOn(l)" (click)="store.toggleLevel(l)" [attr.aria-pressed]="levelOn(l)" [title]="l">
            <span class="dot" [style.background]="levelVar(l)"></span>
            <span class="mono code">{{ code(l) }}</span>
            <span class="mono faint">{{ levelCount(l) }}</span>
          </button>
        }
      </div>

      <div class="anchor" (imtClickOutside)="groupOpen.set(false)">
        <button class="btn tall" (click)="groupOpen.set(!groupOpen())" [attr.aria-expanded]="groupOpen()">
          <imt-icon name="group" [size]="14" />
          <span class="muted">Group by</span>
          <span class="mono">{{ store.effectiveGroupBy().join(' · ') || '—' }}</span>
          <imt-icon name="chevronDown" [size]="12" />
        </button>
        @if (groupOpen()) {
          <div class="popover group-pop">
            <div class="faint small">Instance key — events with equal values form one timeline lane.</div>
            <div class="chosen">
              @for (g of store.effectiveGroupBy(); track g; let i = $index) {
                <span class="chip"><span class="mono">{{ i + 1 }}. {{ g }}</span>
                  <button class="x" [attr.aria-label]="'Remove ' + g" (click)="removeGroupField(g)">×</button></span>
              }
            </div>
            <div class="opts">
              @for (f of groupCandidates(); track f) {
                <button class="opt mono" (click)="addGroupField(f)">+ {{ f }}</button>
              }
            </div>
            <div class="pop-actions">
              <button class="btn ghost" (click)="store.setGroupBy([]); groupOpen.set(false)">Use profile default</button>
            </div>
          </div>
        }
      </div>

      <button class="btn tall" (click)="store.hasRange() && zoom.fit()" [title]="store.hasRange() ? 'Reset to the full range' : 'Drag on the timeline to zoom'">
        <span class="mono">{{ rangeLabel() }}</span>
        @if (store.hasRange()) {
          <span class="faint">({{ rangeSpan() }})</span>
          <imt-icon name="zoomOut" [size]="13" />
        }
      </button>
      <button class="tz" (click)="store.setTimezone(store.timezone() === 'utc' ? 'local' : 'utc')" title="Toggle UTC / local time">{{ tz() }}</button>
    </div>

    <div class="row">
      @for (f of q().filters; track $index; let i = $index) {
        <span class="chip">
          <span class="mono field">{{ f.field }}</span>
          <span class="muted">{{ opLabel(f.op) }}</span>
          @if (f.op !== 'exists' && f.op !== 'missing') { <span class="mono">{{ f.value }}</span> }
          <button class="x" [attr.aria-label]="'Remove filter ' + f.field" (click)="store.removeFilter(i)">×</button>
        </span>
      }
      @for (g of q().groups; track g) {
        <span class="chip">
          <span class="muted">instance</span>
          <span class="mono">{{ g }}</span>
          <button class="x" [attr.aria-label]="'Remove instance filter ' + g" (click)="store.toggleGroup(g)">×</button>
        </span>
      }

      <div class="anchor" (imtClickOutside)="addOpen.set(false)">
        <button class="btn dashed pill" (click)="addOpen.set(!addOpen())" [attr.aria-expanded]="addOpen()">+ Add filter</button>
        @if (addOpen()) {
          <form class="popover add-pop" (submit)="$event.preventDefault(); addFilter()">
            <label class="lbl">Field
              <input class="input mono" list="imt-field-names" [value]="newField()" (input)="newField.set($any($event.target).value)" placeholder="SourceContext" required />
            </label>
            <datalist id="imt-field-names">
              @for (f of allFields(); track f) { <option [value]="f"></option> }
            </datalist>
            <label class="lbl">Operator
              <select class="select" [value]="newOp()" (change)="newOp.set($any($event.target).value)">
                @for (op of ops; track op) { <option [value]="op" [selected]="op === newOp()">{{ opLabel(op) }} ({{ op }})</option> }
              </select>
            </label>
            @if (newOp() !== 'exists' && newOp() !== 'missing') {
              <label class="lbl">Value
                <input class="input mono" [value]="newValue()" (input)="newValue.set($any($event.target).value)" />
              </label>
            }
            <div class="pop-actions">
              <button type="button" class="btn ghost" (click)="addOpen.set(false)">Cancel</button>
              <button type="submit" class="btn primary" [disabled]="!newField()">Add</button>
            </div>
          </form>
        }
      </div>

      @if (hasAnyFilter()) {
        <button class="btn ghost pill" (click)="store.clearQuery()">Clear all</button>
      }
      <span class="spacer"></span>
      <button class="switch-row" (click)="store.setMarkedOnly(!store.markedOnly())" [attr.aria-pressed]="store.markedOnly()">
        <span class="switch" [class.on]="store.markedOnly()"></span> Marked only
      </button>
      <button class="switch-row" (click)="store.setAlwaysShowLifecycle(!store.alwaysShowLifecycle())" [attr.aria-pressed]="store.alwaysShowLifecycle()">
        <span class="switch" [class.on]="store.alwaysShowLifecycle()"></span> Always show start / stop
      </button>
    </div>
  `,
  styles: `
    :host { flex: none; display: flex; flex-direction: column; gap: 8px; padding: 10px 14px; border-bottom: 1px solid var(--border); background: var(--bg-1); }
    .row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .search { flex: 1; min-width: 260px; height: 30px; display: flex; align-items: center; gap: 8px; padding: 0 6px 0 10px; border-radius: 6px; background: var(--bg-3); border: 1px solid var(--border-2); }
    .search:focus-within { border-color: var(--accent); }
    .search.invalid { border-color: var(--lvl-error); }
    .search input { flex: 1; min-width: 0; background: transparent; border: 0; outline: none; font-size: 12px; color: var(--text); }
    .toggle { height: 20px; padding: 0 5px; font-size: 10px; border-radius: 4px; color: var(--muted); border: 1px solid var(--border-3); background: transparent; cursor: pointer; }
    .toggle.on { background: var(--accent); border-color: var(--accent); color: var(--accent-ink); }
    .levels { display: flex; height: 30px; border: 1px solid var(--border-2); border-radius: 6px; overflow: hidden; }
    .lvl { height: 30px; display: flex; align-items: center; gap: 6px; padding: 0 9px; border: 0; border-right: 1px solid var(--border-2); background: var(--bg-2); color: var(--faint); font-size: 12px; cursor: pointer; }
    .lvl:last-child { border-right: 0; }
    .lvl.on { background: var(--bg-active); color: var(--text); }
    .lvl .code, .lvl .faint { font-size: 11px; }
    .tall { height: 30px; }
    .pill { height: 24px; border-radius: 12px; }
    .tz { height: 22px; padding: 0 6px; border: 0; border-radius: 4px; background: var(--bg-active); color: var(--muted); font-size: 11px; cursor: pointer; }
    .anchor { position: relative; }
    .chip .field { color: var(--chip-text); }
    .x { border: 0; background: transparent; color: var(--faint); cursor: pointer; padding: 0 0 0 2px; font-size: 14px; line-height: 1; }
    .x:hover { color: var(--text); }
    .spacer { flex: 1; }
    .switch-row { display: inline-flex; align-items: center; gap: 6px; border: 0; background: transparent; color: var(--muted); font-size: 12px; cursor: pointer; padding: 2px 4px; }
    .group-pop { top: 34px; left: 0; width: 320px; display: flex; flex-direction: column; gap: 8px; }
    .chosen { display: flex; flex-wrap: wrap; gap: 6px; }
    .opts { max-height: 220px; overflow: auto; display: flex; flex-direction: column; }
    .opt { text-align: left; height: 24px; border: 0; border-radius: 4px; background: transparent; color: var(--text-2); cursor: pointer; font-size: 12px; padding: 0 6px; }
    .opt:hover { background: var(--bg-hover); }
    .pop-actions { display: flex; justify-content: flex-end; gap: 6px; }
    .add-pop { top: 30px; left: 0; width: 300px; display: flex; flex-direction: column; gap: 8px; }
    .lbl { display: flex; flex-direction: column; gap: 4px; color: var(--muted); font-size: 11px; }
    .small { font-size: 11px; }
  `,
})
export class FilterBar {
  protected readonly store = inject(WorkspaceStore);
  protected readonly zoom = inject(TimelineZoom);
  protected readonly levels = LEVELS;
  protected readonly ops: FilterOp[] = ['eq', 'ne', 'contains', 'notContains', 'matches', 'exists', 'missing', 'gt', 'lt'];
  protected readonly groupOpen = signal(false);
  protected readonly addOpen = signal(false);
  protected readonly newField = signal('');
  protected readonly newOp = signal<FilterOp>('eq');
  protected readonly newValue = signal('');
  private readonly search = viewChild<ElementRef<HTMLInputElement>>('search');

  protected readonly q = this.store.query;
  protected readonly tz = computed(() => tzLabel(this.store.timezone()));
  protected readonly allFields = computed(() => ['@level', '@message', '@exception', '@template', '@source', ...this.store.fieldNames()]);
  protected readonly groupCandidates = computed(() => {
    const chosen = new Set(this.store.effectiveGroupBy());
    return ['@source', ...this.store.fieldNames()].filter((f) => !chosen.has(f));
  });
  protected readonly regexInvalid = computed(() => {
    const q = this.q();
    if (!q.regex || !q.text) return false;
    try {
      new RegExp(q.text.replace(/\(\?P</g, '(?<'));
      return false;
    } catch {
      return true;
    }
  });
  protected readonly hasAnyFilter = computed(() => {
    const q = this.q();
    return !!(q.text || q.levels.length || q.filters.length || q.groups.length || q.markers.length || q.from != null || q.to != null);
  });
  protected readonly rangeLabel = computed(() => {
    const tz = this.store.timezone();
    const q = this.q();
    const s = this.store.summary();
    if (q.from == null && q.to == null) {
      return s?.from != null ? `${formatDateTime(s.from, tz)} → ${formatDateTime(s.to, tz)}` : 'All time';
    }
    return `${formatDateTime(q.from ?? s?.from ?? null, tz)} → ${formatDateTime(q.to ?? s?.to ?? null, tz)}`;
  });
  protected readonly rangeSpan = computed(() => {
    const q = this.q();
    return q.from != null && q.to != null ? formatDuration(q.to - q.from) : '';
  });

  constructor() {
    effect(() => {
      if (this.store.focusSearchNonce() === 0) return;
      untracked(() => setTimeout(() => this.search()?.nativeElement.focus()));
    });
  }

  protected levelOn(l: Level): boolean {
    return this.q().levels.includes(l);
  }
  protected levelVar(l: Level): string {
    return LEVEL_VARS[l];
  }
  protected code(l: Level): string {
    return LEVEL_CODES[l];
  }
  protected levelCount(l: Level): string {
    return formatCompact(this.store.summary()?.levels[l] ?? 0);
  }
  protected opLabel(op: FilterOp): string {
    return OP_LABELS[op];
  }

  protected addGroupField(f: string): void {
    this.store.setGroupBy([...this.store.effectiveGroupBy(), f]);
  }
  protected removeGroupField(f: string): void {
    this.store.setGroupBy(this.store.effectiveGroupBy().filter((g) => g !== f));
  }

  protected addFilter(): void {
    const field = this.newField().trim();
    if (!field) return;
    const op = this.newOp();
    this.store.addFilter(op === 'exists' || op === 'missing' ? { field, op } : { field, op, value: this.newValue() });
    this.addOpen.set(false);
    this.newValue.set('');
  }
}
