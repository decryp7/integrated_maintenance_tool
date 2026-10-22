import { ChangeDetectionStrategy, Component, computed, effect, ElementRef, inject, signal, untracked, viewChild } from '@angular/core';
import { FilterOp, LEVELS, Level } from '../core/api.types';
import { formatCompact, formatDateTime, formatDuration, tzLabel } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { SmartSearchStore } from '../core/workspace/smart-search.store';
import { LevelCountsStore } from '../core/workspace/level-counts.store';
import { plural } from '../i18n/locale';
import { ClickOutside } from '../shared/click-outside';
import { LEVEL_CODES, LEVEL_VARS } from '../shared/colors';
import { Icon } from '../shared/icon';
import { Switch } from '../shared/switch';
import { TimelineZoom } from '../timeline/zoom';

const OP_LABELS: Record<FilterOp, string> = {
  eq: '=',
  ne: '≠',
  contains: '~',
  notContains: '!~',
  matches: $localize`:Filter operator|@@workspace.filterBar.op.matches:matches`,
  exists: $localize`:Filter operator|@@workspace.filterBar.op.exists:exists`,
  missing: $localize`:Filter operator|@@workspace.filterBar.op.missing:missing`,
  gt: '>',
  lt: '<',
};

@Component({
  selector: 'imt-filter-bar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, ClickOutside, Switch],
  template: `
    <div class="row">
      <label class="search" [class.invalid]="regexInvalid()">
        <imt-icon name="search" [size]="14" class="faint" />
        <input
          #search
          class="mono"
          aria-label="Search messages and exceptions"
          i18n-aria-label="@@workspace.filterBar.search"
          [placeholder]="searchPlaceholder()"
          [value]="store.textInput()"
          (input)="store.setTextInput($any($event.target).value)"
          (keydown.escape)="store.setTextInput('')"
        />
        @if (smart.added() > 0) {
          <span class="partial-note mono" role="status" data-testid="partial-note" [title]="partialNote()">+{{ smart.added() }}</span>
        }
        <button class="toggle mono" [class.on]="q().regex" (click)="store.setRegex(!q().regex)" title="Regular expression" i18n-title="@@workspace.filterBar.regex" [attr.aria-pressed]="q().regex">.*</button>
        <button class="toggle mono" [class.on]="q().caseSensitive" (click)="store.setCaseSensitive(!q().caseSensitive)" title="Match case" i18n-title="@@workspace.filterBar.matchCase" [attr.aria-pressed]="q().caseSensitive">Aa</button>
      </label>

      <div class="levels" role="group" aria-label="Levels" i18n-aria-label="@@workspace.filterBar.levels">
        @for (l of levels; track l) {
          <button class="lvl" [attr.data-level]="l" [class.on]="levelOn(l)" [class.none]="levelMatching(l) === 0" (click)="store.toggleLevel(l)" [attr.aria-pressed]="levelOn(l)" [title]="levelTitle(l)">
            <span class="dot" [style.background]="levelVar(l)"></span>
            <span class="mono code">{{ code(l) }}</span>
            <span class="mono faint">{{ levelCount(l) }}</span>
          </button>
        }
      </div>

      <button class="btn tall range" (click)="store.hasRange() && zoom.fit()" [title]="rangeLabel() + ' · ' + rangeTitle()">
        <span class="mono ends"><span class="end">{{ rangeEnds()[0] }}</span>@if (rangeEnds()[1]) {<span> → </span><span class="end">{{ rangeEnds()[1] }}</span>}</span>
        @if (store.hasRange()) {
          <span class="faint">({{ rangeSpan() }})</span>
          <imt-icon name="zoomOut" [size]="13" />
        }
      </button>
      <button class="tz" (click)="store.setTimezone(store.timezone() === 'utc' ? 'local' : 'utc')" title="Toggle UTC / local time" i18n-title="@@workspace.filterBar.toggleTz">{{ tz() }}</button>
      <div class="anchor" (imtClickOutside)="addOpen.set(false)">
        <button class="btn dashed pill" (click)="addOpen.set(!addOpen())" [attr.aria-expanded]="addOpen()" i18n="@@workspace.filterBar.addFilter">+ Add filter</button>
        @if (addOpen()) {
          <form class="popover add-pop" (submit)="$event.preventDefault(); addFilter()">
            <label class="lbl"><ng-container i18n="@@workspace.filterBar.field">Field</ng-container>
              <input class="input mono" list="imt-field-names" [value]="newField()" (input)="newField.set($any($event.target).value)" placeholder="SourceContext" required />
            </label>
            <datalist id="imt-field-names">
              @for (f of allFields(); track f) { <option [value]="f"></option> }
            </datalist>
            <label class="lbl"><ng-container i18n="@@workspace.filterBar.operator">Operator</ng-container>
              <select class="select" [value]="newOp()" (change)="newOp.set($any($event.target).value)">
                @for (op of ops; track op) { <option [value]="op" [selected]="op === newOp()">{{ opLabel(op) }} ({{ op }})</option> }
              </select>
            </label>
            @if (newOp() !== 'exists' && newOp() !== 'missing') {
              <label class="lbl"><ng-container i18n="@@workspace.filterBar.value">Value</ng-container>
                <input class="input mono" [value]="newValue()" (input)="newValue.set($any($event.target).value)" />
              </label>
            }
            <div class="pop-actions">
              <button type="button" class="btn ghost" (click)="addOpen.set(false)" i18n="@@workspace.filterBar.cancel">Cancel</button>
              <button type="submit" class="btn primary" [disabled]="!newField()" i18n="@@workspace.filterBar.add">Add</button>
            </div>
          </form>
        }
      </div>

      @if (hasAnyFilter()) {
        <button class="btn ghost pill" (click)="store.clearQuery()" i18n="@@workspace.filterBar.clearAll">Clear all</button>
      }
      <span class="spacer"></span>
      <div class="anchor" (imtClickOutside)="optionsOpen.set(false)" (keydown.escape)="optionsOpen.set(false)">
        <button class="btn tall" data-testid="view-options" aria-haspopup="dialog" [attr.aria-expanded]="optionsOpen()" aria-controls="imt-view-options" (click)="optionsOpen.set(!optionsOpen())">
          <imt-icon name="group" [size]="14" />
          <span i18n="@@workspace.filterBar.viewOptions">View options</span>
          <imt-icon name="chevronDown" [size]="12" />
        </button>
        @if (optionsOpen()) {
          <div class="popover options-pop" id="imt-view-options" role="dialog" aria-label="View options" i18n-aria-label="@@workspace.filterBar.viewOptionsAria">
            <div class="opt-section">
              <div class="opt-head"><span class="muted" i18n="@@workspace.filterBar.groupLanesBy">Group events into timeline lanes by</span> <span class="mono">{{ store.effectiveGroupBy().join(' · ') || '—' }}</span></div>
              <div class="faint small" i18n="@@workspace.filterBar.groupLanesHelp">Events with equal values in these fields form one lane.</div>
              <div class="chosen">
                @for (g of store.effectiveGroupBy(); track g; let i = $index) {
                  <span class="chip"><span class="mono">{{ i + 1 }}. {{ g }}</span>
                    <button class="x" [attr.aria-label]="removeLabel(g)" (click)="removeGroupField(g)" [attr.title]="removeLabel(g)">×</button></span>
                }
              </div>
              <div class="opts" role="group" aria-label="Add a field to the group-by" i18n-aria-label="@@workspace.filterBar.groupCandidates">
                @for (f of groupCandidates(); track f) {
                  <button class="opt mono" (click)="addGroupField(f)">+ {{ f }}</button>
                }
              </div>
              <div class="pop-actions">
                <button class="btn ghost" (click)="store.setGroupBy([])" i18n="@@workspace.filterBar.profileDefault">Use profile default</button>
              </div>
            </div>
            <div class="opt-section switches">
              <imt-switch class="sw" size="small" [checked]="store.markedOnly()" (checkedChange)="store.setMarkedOnly($event)" i18n="@@workspace.filterBar.markedOnly">Marked only</imt-switch>
              <imt-switch class="sw" size="small" [checked]="store.alwaysShowLifecycle()" (checkedChange)="store.setAlwaysShowLifecycle($event)" i18n="@@workspace.filterBar.alwaysLifecycle">Always show start / stop</imt-switch>
            </div>
          </div>
        }
      </div>
    </div>

    @if (q().filters.length || q().groups.length) {
    <div class="row" role="group" aria-label="Filters" i18n-aria-label="@@workspace.filterBar.filters">
      @for (f of q().filters; track $index; let i = $index) {
        <span class="chip">
          <span class="mono field">{{ f.field }}</span>
          <span class="muted">{{ opLabel(f.op) }}</span>
          @if (f.op !== 'exists' && f.op !== 'missing') { <span class="mono">{{ f.value }}</span> }
          <button class="x" [attr.aria-label]="removeFilterLabel(f.field)" (click)="store.removeFilter(i)" [attr.title]="removeFilterLabel(f.field)">×</button>
        </span>
      }
      @for (g of q().groups; track g) {
        <span class="chip">
          <span class="muted" i18n="@@workspace.filterBar.instance">instance</span>
          <span class="mono">{{ g }}</span>
          <button class="x" [attr.aria-label]="removeInstanceLabel(g)" (click)="store.toggleGroup(g)" [attr.title]="removeInstanceLabel(g)">×</button>
        </span>
      }

    </div>
    }
  `,
  styles: `
    :host { flex: none; display: flex; flex-direction: column; gap: 8px; padding: 10px 14px; border-bottom: 1px solid var(--border); background: var(--bg-1); }
    .row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    .row:first-child { flex-wrap: nowrap; }
    .search { flex: 1; min-width: 200px; height: 30px; display: flex; align-items: center; gap: 8px; padding: 0 6px 0 10px; border-radius: 6px; background: var(--bg-3); border: 1px solid var(--border-2); }
    .search:focus-within { border-color: var(--accent); }
    .search.invalid { border-color: var(--lvl-error); }
    .partial-note { flex: none; font-size: var(--fs-xs); color: var(--text-2); padding: 1px 6px; border-radius: 999px; border: 1px dashed var(--border-2); }
    .search input { flex: 1; min-width: 0; background: transparent; border: 0; outline: none; font-size: var(--fs-sm); color: var(--text); }
    .toggle { height: 20px; padding: 0 5px; font-size: var(--fs-2xs); border-radius: 4px; color: var(--muted); border: 1px solid var(--border-3); background: transparent; cursor: pointer; }
    .toggle.on { background: var(--accent); border-color: var(--accent); color: var(--accent-ink); }
    .levels { flex: none; display: flex; height: 30px; border: 1px solid var(--border-2); border-radius: 6px; overflow: hidden; }
    .lvl { height: 30px; display: flex; align-items: center; gap: 6px; padding: 0 9px; border: 0; border-right: 1px solid var(--border-2); background: var(--bg-2); color: var(--faint); font-size: var(--fs-sm); cursor: pointer; }
    .lvl:last-child { border-right: 0; }
    .lvl.on { background: var(--bg-active); color: var(--text); }
    .lvl .code, .lvl .faint { font-size: var(--fs-xs); }
    .tall { height: 30px; flex: none; }
    /* The range shrinks (with an ellipsis) before anything else in the one-row bar. */
    .range { flex: 0 1 auto; min-width: 80px; overflow: hidden; }
    .range .ends { display: flex; gap: 4px; min-width: 0; overflow: hidden; white-space: nowrap; font-size: var(--fs-xs); }
    .range .end { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
    .lvl.none .code, .lvl.none .faint { opacity: 0.55; }
    .pill { height: 24px; border-radius: 12px; flex: none; }
    .tz { height: 22px; padding: 0 6px; border: 0; border-radius: 4px; background: var(--bg-active); color: var(--muted); font-size: var(--fs-xs); cursor: pointer; }
    .anchor { position: relative; }
    .chip .field { color: var(--chip-text); }
    .x { border: 0; background: transparent; color: var(--faint); cursor: pointer; padding: 0 0 0 2px; font-size: var(--fs-lg); line-height: 1; }
    .x:hover { color: var(--text); }
    .spacer { flex: 1; }
    .options-pop { top: 34px; right: 0; width: 340px; display: flex; flex-direction: column; gap: 12px; }
    .opt-section { display: flex; flex-direction: column; gap: 8px; }
    .opt-head { display: flex; flex-wrap: wrap; gap: 6px; align-items: baseline; font-size: var(--fs-sm); }
    .switches { border-top: 1px solid var(--border); padding-top: 10px; }
    .sw { color: var(--muted); }
    .chosen { display: flex; flex-wrap: wrap; gap: 6px; }
    .opts { max-height: 220px; overflow: auto; display: flex; flex-direction: column; }
    /* flex: none: in the scrolling column the rows would otherwise shrink below the 24 px target size. */
    .opt { flex: none; white-space: nowrap; text-align: left; min-height: 24px; border: 0; border-radius: 4px; background: transparent; color: var(--text-2); cursor: pointer; font-size: var(--fs-sm); padding: 0 6px; }
    .opt:hover { background: var(--bg-hover); }
    .pop-actions { display: flex; justify-content: flex-end; gap: 6px; }
    .add-pop { top: 30px; left: 0; width: 300px; display: flex; flex-direction: column; gap: 8px; }
    .lbl { display: flex; flex-direction: column; gap: 4px; color: var(--muted); font-size: var(--fs-xs); }
    .small { font-size: var(--fs-xs); }
  `,
})
export class FilterBar {
  protected readonly store = inject(WorkspaceStore);
  protected readonly smart = inject(SmartSearchStore);
  private readonly levelCounts = inject(LevelCountsStore);
  /** What the "+N" in the search box means: the engine added partial matches to the word matches. */
  protected readonly partialNote = computed(() => {
    const n = this.smart.added();
    return plural(n, {
      one: $localize`:@@workspace.filterBar.partialOne:${n}:count: more partial match added: the text appears inside a word`,
      other: $localize`:@@workspace.filterBar.partialMany:${n}:count: more partial matches added: the text appears inside words`,
    });
  });
  protected readonly zoom = inject(TimelineZoom);
  protected readonly levels = LEVELS;
  protected readonly ops: FilterOp[] = ['eq', 'ne', 'contains', 'notContains', 'matches', 'exists', 'missing', 'gt', 'lt'];
  protected readonly optionsOpen = signal(false);
  protected readonly addOpen = signal(false);
  protected readonly newField = signal('');
  protected readonly newOp = signal<FilterOp>('eq');
  protected readonly newValue = signal('');
  private readonly search = viewChild<ElementRef<HTMLInputElement>>('search');

  protected readonly q = this.store.query;
  protected readonly searchPlaceholder = computed(() =>
    this.q().regex
      ? $localize`:@@workspace.filterBar.regexPlaceholder:Regular expression`
      : $localize`:@@workspace.filterBar.searchPlaceholderShort:Search…`,
  );
  protected readonly rangeTitle = computed(() =>
    this.store.hasRange()
      ? $localize`:@@workspace.filterBar.resetRange:Reset to the full range`
      : $localize`:@@workspace.filterBar.dragToZoom:Drag on the timeline to zoom`,
  );
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
  /** Both ends of the range; each truncates on its own, so a narrow bar still shows where the range starts and ends. */
  protected readonly rangeEnds = computed<[string, string]>(() => {
    const tz = this.store.timezone();
    const q = this.q();
    const s = this.store.summary();
    if (q.from == null && q.to == null && s?.from == null) return [$localize`:@@workspace.filterBar.allTime:All time`, ''];
    return [formatDateTime(q.from ?? s?.from ?? null, tz), formatDateTime(q.to ?? s?.to ?? null, tz)];
  });
  /** The range as one line, for the button's tooltip. */
  protected readonly rangeLabel = computed(() => this.rangeEnds().filter(Boolean).join(' → '));
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
  /** Events of level `l` matching the other filters; null while nothing else narrows the result. */
  protected levelMatching(l: Level): number | null {
    return this.levelCounts.matching()?.[l] ?? null;
  }
  /** "403", or "12/403" when the other filters narrow the result. */
  protected levelCount(l: Level): string {
    const total = formatCompact(this.store.summary()?.levels[l] ?? 0);
    const matching = this.levelMatching(l);
    return matching === null ? total : `${formatCompact(matching)}/${total}`;
  }
  protected levelTitle(l: Level): string {
    const matching = this.levelMatching(l);
    if (matching === null) return l;
    const total = this.store.summary()?.levels[l] ?? 0;
    return $localize`:@@workspace.filterBar.levelMatching:${l}:level:: ${matching}:matching: of ${total}:total: match the other filters`;
  }
  protected opLabel(op: FilterOp): string {
    return OP_LABELS[op];
  }

  protected removeLabel(name: string): string {
    return $localize`:@@workspace.filterBar.removeAria:Remove ${name}:name:`;
  }
  protected removeFilterLabel(field: string): string {
    return $localize`:@@workspace.filterBar.removeFilterAria:Remove filter ${field}:field:`;
  }
  protected removeInstanceLabel(group: string): string {
    return $localize`:@@workspace.filterBar.removeInstanceAria:Remove instance filter ${group}:group:`;
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
